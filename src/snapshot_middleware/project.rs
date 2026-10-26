use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{BTreeMap, HashMap, VecDeque},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use memofs::Vfs;
use rbx_dom_weak::{
    types::{Attributes, Ref, Variant},
    ustr, HashMapExt as _, Instance, Ustr, UstrMap,
};
use rbx_reflection::ClassTag;

use crate::{
    project::{PathNode, Project, ProjectFormatVersion, ProjectNode},
    resolution::UnresolvedValue,
    snapshot::{
        InstanceContext, InstanceMetadata, InstanceSnapshot, InstanceWithMeta, InstigatingSource,
        PathIgnoreRule, SyncRule,
    },
    snapshot_middleware::Middleware,
    syncback::{filter_properties, FsSnapshot, SyncbackReturn, SyncbackSnapshot},
    variant_eq::variant_eq,
    RojoRef,
};

use super::snapshot_from_vfs;

thread_local! {
    static ACTIVE_PROJECTS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

struct ProjectSnapshotGuard;

impl ProjectSnapshotGuard {
    fn enter(vfs: &Vfs, path: &Path) -> anyhow::Result<Self> {
        let path = vfs
            .canonicalize(path)
            .with_context(|| format!("Could not resolve project path {}", path.display()))?;
        Self::enter_key(path)
    }

    fn enter_key(path: PathBuf) -> anyhow::Result<Self> {
        ACTIVE_PROJECTS.with(|active| {
            let mut active = active.borrow_mut();
            if active.contains(&path) {
                let chain = active
                    .iter()
                    .chain(std::iter::once(&path))
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                bail!("Circular project reference: {chain}");
            }
            active.push(path);
            Ok(Self)
        })
    }
}

impl Drop for ProjectSnapshotGuard {
    fn drop(&mut self) {
        ACTIVE_PROJECTS.with(|active| {
            active.borrow_mut().pop();
        });
    }
}

pub fn snapshot_project(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    // Keep traversal state outside persisted snapshot contexts so later file
    // updates and independent aliases can load the same project again.
    let _guard = ProjectSnapshotGuard::enter(vfs, path)?;
    let project = Project::load_exact(vfs, path, Some(name))
        .with_context(|| format!("File was not a valid Rojo project: {}", path.display()))?;
    snapshot_project_contents(context, vfs, &project)
}

pub(crate) fn snapshot_loaded_project(
    context: &InstanceContext,
    vfs: &Vfs,
    project: &Project,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let _guard = ProjectSnapshotGuard::enter(vfs, &project.file_location)?;
    snapshot_project_contents(context, vfs, project)
}

pub(crate) fn snapshot_stdin_project(
    context: &InstanceContext,
    vfs: &Vfs,
    project: &Project,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    // stdin has no file to canonicalize. Its parent is an actual source base;
    // retain a unique traversal key without creating a temporary project file.
    let parent = vfs.canonicalize(project.folder_location())?;
    let key = parent.join(
        project
            .file_location
            .file_name()
            .context("stdin origin has no filename")?,
    );
    let _guard = ProjectSnapshotGuard::enter_key(key)?;
    snapshot_project_contents(context, vfs, project)
}

fn snapshot_project_contents(
    context: &InstanceContext,
    vfs: &Vfs,
    project: &Project,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let path = &project.file_location;
    let project_name = project
        .name
        .as_deref()
        .with_context(|| format!("Project is missing a name: {}", path.display()))?;

    let incoming_ignore_empty = context.ignore_empty_directories;
    let incoming_policy = context.ignore_unknown_descendants;
    let incoming_authority = context.sync_authority.clone();
    let mut context = context.clone();
    if let Some(authority) = &project.sync_authority {
        context.sync_authority = Some(std::sync::Arc::new(authority.clone()));
    }
    if context.ignore_unknown_descendants.is_none()
        && project.format_version == Some(ProjectFormatVersion::V2)
    {
        context.ignore_unknown_descendants = Some(true);
    }
    context.clear_sync_rules();
    context.rule_owner = None;
    if let Some(ignore_empty) = project.ignore_empty_directories {
        context.ignore_empty_directories = ignore_empty;
    }

    let rules = project.glob_ignore_paths.iter().map(|glob| PathIgnoreRule {
        glob: glob.clone(),
        base_path: project.folder_location().to_path_buf(),
    });

    let sync_rules = project.sync_rules.iter().map(|rule| SyncRule {
        base_path: project.folder_location().to_path_buf(),
        ..rule.clone()
    });

    context.add_sync_rules(sync_rules);
    context.add_path_ignore_rules(rules);
    context.set_emit_legacy_scripts(project.effective_emit_legacy_scripts());

    match snapshot_project_node(&context, path, project_name, &project.tree, vfs, None)? {
        Some(found_snapshot) => {
            let mut snapshot = found_snapshot;
            // Setting the instigating source to the project file path is a little
            // coarse.
            //
            // Ideally, we'd only snapshot the project file if the project file
            // actually changed. Because Rojo only has the concept of one
            // relevant path -> snapshot path mapping per instance, we pick the more
            // conservative approach of snapshotting the project file if any
            // relevant paths changed.
            snapshot.metadata.instigating_source = Some(path.to_path_buf().into());
            // Root refreshes must inherit from the enclosing project again if
            // this project's explicit override was removed in the meantime.
            snapshot.metadata.context.ignore_empty_directories = incoming_ignore_empty;
            snapshot.metadata.context.ignore_unknown_descendants = incoming_policy;
            snapshot.metadata.context.sync_authority = incoming_authority;

            // Mark this snapshot (the root node of the project file) as being
            // related to the project file.
            //
            // We SHOULD NOT mark the project file as a relevant path for any
            // nodes that aren't roots. They'll be updated as part of the project
            // file being updated.
            snapshot.metadata.relevant_paths.push(path.to_path_buf());

            Ok(Some(snapshot))
        }
        None => Ok(None),
    }
}

pub fn snapshot_project_node(
    context: &InstanceContext,
    project_path: &Path,
    instance_name: &str,
    node: &ProjectNode,
    vfs: &Vfs,
    parent_class: Option<&str>,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let project_folder = project_path.parent().unwrap();

    let incoming_policy = context.ignore_unknown_descendants;
    let mut node_context = context.clone();
    if let Some(policy) = node.ignore_unknown_descendants {
        node_context.ignore_unknown_descendants = Some(policy);
    }
    let context = &node_context;

    let mut class_name_from_path = None;
    let mut snapshot_id = Ref::none();

    let name = Cow::Owned(instance_name.to_owned());
    let mut properties = UstrMap::new();
    let mut children = Vec::new();
    let mut metadata = InstanceMetadata::new().context(context);

    if let Some(path_node) = &node.path {
        let path_snapshot = if let PathNode::Multiple(paths) = path_node {
            Some(super::path_arrays::snapshot_paths(
                context,
                vfs,
                project_path,
                instance_name,
                paths,
            )?)
        } else {
            let path = path_node.path();

            // If the path specified in the project is relative, we assume it's
            // relative to the folder that the project is in, project_folder.
            let full_path = if path.is_relative() {
                Cow::Owned(project_folder.join(path))
            } else {
                Cow::Borrowed(path)
            };

            let mut source_context = context.clone();
            source_context.rule_owner = Some(std::sync::Arc::new(super::RuleOwner {
                path: full_path.to_path_buf(),
                class_name: node.class_name,
                mask: node.rule_default_mask().clone(),
            }));
            snapshot_from_vfs(&source_context, vfs, &full_path)?
        };
        if let Some(mut snapshot) = path_snapshot {
            // References inside a model still point at its original root ID.
            // Keep that mapping through every project layer before patching.
            snapshot_id = snapshot.snapshot_id;
            super::rule_defaults::project_mask(&mut snapshot, node.rule_default_mask())?;
            class_name_from_path = Some(snapshot.class_name);

            // Properties from the snapshot are pulled in unchanged, and
            // overridden by properties set on the project node.
            properties.reserve(snapshot.properties.len());
            for (key, value) in snapshot.properties.into_iter() {
                properties.insert(key, value);
            }

            // The snapshot's children will be merged with the children defined
            // in the project node, if there are any.
            children.reserve(snapshot.children.len());
            for child in snapshot.children.into_iter() {
                children.push(child);
            }

            // Take the snapshot's metadata as-is, which will be mutated later
            // on.
            metadata = snapshot.metadata;
        }
    }

    let class_name_from_inference = infer_class_name(&name, parent_class);

    let class_name = match (
        node.class_name,
        class_name_from_path,
        class_name_from_inference,
        &node.path,
    ) {
        // These are the easy, happy paths!
        (Some(project), None, None, _) => project,
        (None, Some(path), None, _) => path,
        (None, None, Some(inference), _) => inference,

        // If the user specifies a class name, but there's an inferred class
        // name, we prefer the name listed explicitly by the user.
        (Some(project), None, Some(_), _) => project,

        // If the user has a $path pointing to a folder and we're able to infer
        // a class name, let's use the inferred name. If the path we're pointing
        // to isn't a folder, though, that's a user error.
        (None, Some(path), Some(inference), _) => {
            if path == "Folder" {
                inference
            } else {
                path
            }
        }

        (Some(project), Some(path), _, _) => {
            if path == "Folder" || project == path {
                project
            } else {
                bail!(
                    "ClassName for Instance \"{}\" was specified in both the project file (as \"{}\") and from the filesystem (as \"{}\").\n\
                     If $className and $path are both set, their classes must match or $path must refer to a Folder.
                     \n\
                     Project path: {}\n\
                     Filesystem path: {}\n",
                    instance_name,
                    project,
                    path,
                    project_path.display(),
                    node.path.as_ref().unwrap().path().display()
                );
            }
        }

        (None, None, None, Some(PathNode::Optional(_))) => {
            return Ok(None);
        }

        (_, None, _, Some(PathNode::Required(path))) => {
            anyhow::bail!(
                "Rojo project referred to a file using $path that could not be turned into a Roblox Instance by Rojo.\n\
                Check that the file exists and is a file type known by Rojo.\n\
                \n\
                Project path: {}\n\
                File $path: {}",
                project_path.display(),
                path.display(),
            );
        }

        (_, None, _, Some(PathNode::Multiple(_))) => {
            unreachable!("array snapshots always have a root")
        }

        (None, None, None, None) => {
            bail!(
                "Instance \"{}\" is missing some required information.\n\
                 One of the following must be true:\n\
                 - $className must be set to the name of a Roblox class\n\
                 - $path must be set to a path of an instance\n\
                 - The instance must be a known service, like ReplicatedStorage\n\
                 \n\
                 Project path: {}",
                instance_name,
                project_path.display(),
            );
        }
    };

    for (child_name, child_project_node) in &node.children {
        if matches!(node.path, Some(PathNode::Multiple(_))) {
            if let Some(child) = children.iter().find(|child| child.name == *child_name) {
                bail!(
                    "Explicit project child {child_name} in {} conflicts with array source {:?}",
                    project_path.display(),
                    child.metadata.instigating_source
                );
            }
        }
        if let Some(child) = snapshot_project_node(
            context,
            project_path,
            child_name,
            child_project_node,
            vfs,
            Some(&class_name),
        )? {
            children.push(child);
        }
    }

    for (key, unresolved) in &node.properties {
        let value = unresolved
            .clone()
            .resolve(&class_name, key)
            .with_context(|| {
                format!(
                    "Unresolvable property in project at path {}",
                    project_path.display()
                )
            })?;

        match key.as_str() {
            "Name" | "Parent" => {
                log::warn!(
                    "Property '{}' cannot be set manually, ignoring. Attempted to set in '{}' at {}",
                    key,
                    instance_name,
                    project_path.display()
                );
                continue;
            }

            _ => {}
        }

        properties.insert(*key, value);
    }

    if !node.attributes.is_empty() {
        let mut attributes = if metadata.rule_defaults.is_some() {
            match properties.get(&ustr("Attributes")) {
                Some(Variant::Attributes(attrs)) => attrs.clone(),
                _ => Attributes::new(),
            }
        } else {
            Attributes::new()
        };

        for (key, unresolved) in &node.attributes {
            let value = unresolved.clone().resolve_unambiguous().with_context(|| {
                format!(
                    "Unresolvable attribute in project at path {}",
                    project_path.display()
                )
            })?;

            attributes.insert(key.clone(), value);
        }

        properties.insert("Attributes".into(), attributes.into());
    }

    // If the user specified $ignoreUnknownInstances, overwrite the existing
    // value.
    //
    // If the user didn't specify it AND $path was not specified (meaning
    // there's no existing value we'd be stepping on from a project file or meta
    // file), set it to true.
    if let Some(ignore) = node.ignore_unknown_instances {
        metadata.ignore_unknown_instances = ignore;
    } else if node.path.is_none() {
        // TODO: Introduce a strict mode where $ignoreUnknownInstances is never
        // set implicitly.
        metadata.ignore_unknown_instances = true;
    }

    if let Some(id) = &node.id {
        metadata.specified_id = Some(RojoRef::new(id.clone()))
    }

    if let Some(state) = &mut metadata.rule_defaults {
        state
            .explicit_properties
            .extend(node.properties.keys().copied());
        state
            .explicit_attributes
            .extend(node.attributes.keys().cloned());
        state.explicit_class |= node.class_name.is_some();
        state.explicit_ignore = node.ignore_unknown_instances.or(state.explicit_ignore);
    }

    // Re-snapshotting must reapply this node's current option, rather than
    // remembering an override that may have been removed from the project.
    metadata.context.ignore_unknown_descendants = incoming_policy;
    metadata.instigating_source = Some(InstigatingSource::ProjectNode {
        path: project_path.to_path_buf(),
        name: instance_name.to_string(),
        node: Box::new(node.clone()),
        parent_class: parent_class.map(|name| name.to_owned()),
    });

    Ok(Some(InstanceSnapshot {
        snapshot_id,
        name,
        class_name,
        properties,
        children,
        metadata,
    }))
}

pub fn syncback_project<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let old_inst = snapshot
        .old_inst()
        .expect("projects should always exist in both trees");
    // Generally, the path of a project is the first thing added to the relevant
    // paths. So, we take the last one.
    let project_path = old_inst
        .metadata()
        .relevant_paths
        .last()
        .expect("all projects should have a relevant path");
    let vfs = snapshot.vfs();

    log::debug!("Reloading project {} from vfs", project_path.display(),);
    let mut project = Project::load_exact(vfs, project_path, None)?;
    let base_path = project.folder_location().to_path_buf();

    // Sync rules for this project do not have their base rule set but it is
    // important when performing syncback on other projects.
    for rule in &mut project.sync_rules {
        rule.base_path.clone_from(&base_path)
    }

    let mut descendant_snapshots = Vec::new();
    let mut removed_descendants = Vec::new();

    let mut ref_to_path_map = HashMap::new();
    let mut old_child_map = HashMap::new();
    let mut new_child_map = HashMap::new();

    let mut node_changed_map = Vec::new();
    // Tracks whether any stale default-valued properties were removed from
    // project nodes. If so, we must reserialize even if
    // project_node_should_reserialize wouldn't otherwise detect a change
    // (it only compares node properties forward, not in reverse).
    let mut removed_stale_properties = false;
    let mut node_queue = VecDeque::with_capacity(1);
    node_queue.push_back((&mut project.tree, old_inst, snapshot.new_inst()));

    while let Some((node, old_inst, new_inst)) = node_queue.pop_front() {
        log::debug!("Processing node {}", old_inst.name());
        if old_inst.class_name() != new_inst.class {
            anyhow::bail!(
                "Cannot change the class of {} in project file {}.\n\
                Current class is {}, it is a {} in the input file.",
                old_inst.name(),
                project_path.display(),
                old_inst.class_name(),
                new_inst.class
            );
        }

        // TODO handle meta.json files in this branch. Right now, we perform
        // syncback if a node has `$path` set but the Middleware aren't aware
        // that the Instances they're running on originate in a project.json.
        // As a result, the `meta.json` syncback code is hardcoded to not work
        // if the Instance originates from a project file. However, we should
        // ideally use a .meta.json over the project node if it exists already.
        if node.path.is_some() {
            // Since the node has a path, we have to run syncback on it.
            let node_path = node.path.as_ref().map(PathNode::path).expect(
                "Project nodes with a path must have a path \
                If you see this message, something went seriously wrong. Please report it.",
            );
            let full_path = if node_path.is_absolute() {
                node_path.to_path_buf()
            } else {
                base_path.join(node_path)
            };

            let middleware = match Middleware::middleware_for_path(
                snapshot.vfs(),
                &project.sync_rules,
                &full_path,
            )? {
                Some(middleware) => middleware,
                // The only way this can happen at this point is if the path does
                // not exist on the file system or there's no middleware for it.
                None => anyhow::bail!(
                    "path does not exist or could not be turned into a file Rojo understands: {}",
                    full_path.display()
                ),
            };

            descendant_snapshots.push(
                snapshot
                    .with_new_path(full_path.clone(), new_inst.referent(), Some(old_inst.id()))
                    .middleware(middleware),
            );

            // We only want to set properties if it needs it.
            if !middleware.handles_own_properties() {
                let property_snapshot = snapshot
                    .with_new_path(full_path.clone(), new_inst.referent(), Some(old_inst.id()))
                    .middleware(middleware);
                removed_stale_properties |=
                    project_node_property_syncback_path(&property_snapshot, new_inst, node)?;
            }
            ref_to_path_map.insert(new_inst.referent(), full_path);
        } else {
            removed_stale_properties |=
                project_node_property_syncback_no_path(snapshot, new_inst, node);
        }

        for child_ref in new_inst.children() {
            let child = snapshot
                .get_new_instance(*child_ref)
                .expect("all children of Instances should be in new DOM");
            // Without a source path, only explicitly declared children belong
            // to this project. Studio-only siblings must not affect matching.
            if node.path.is_none() && !node.children.contains_key(&child.name) {
                continue;
            }
            if new_child_map.insert(&child.name, child).is_some() {
                anyhow::bail!(
                    "Instances that are direct children of an Instance that is made by a project file \
                    must have a unique name.\nThe child '{}' of '{}' is duplicated in the place file.", child.name, old_inst.name()
                );
            }
        }
        for child_ref in old_inst.children() {
            let child = snapshot
                .get_old_instance(*child_ref)
                .expect("all children of Instances should be in old DOM");
            if old_child_map.insert(child.name(), child).is_some() {
                anyhow::bail!(
                    "Instances that are direct children of an Instance that is made by a project file \
                    must have a unique name.\nThe child '{}' of '{}' is duplicated on the file system.", child.name(), old_inst.name()
                );
            }
        }

        // This loop does basic matching of Instance children to the node's
        // children. It ensures that `new_child_map` and `old_child_map` will
        // only contain Instances that don't belong to the project after this.
        for (child_name, child_node) in &mut node.children {
            // If a node's path is optional, we want to skip it if the path
            // doesn't exist since it isn't in the current old DOM.
            if let Some(path) = &child_node.path {
                if path.is_optional() {
                    let real_path = if path.path().is_absolute() {
                        path.path().to_path_buf()
                    } else {
                        base_path.join(path.path())
                    };
                    if !real_path.exists() {
                        log::warn!(
                            "Skipping node '{child_name}' of project because it is optional and not present on the disk.\n\
                            If this is not deliberate, please create a file or directory at {}", real_path.display()
                        );
                        continue;
                    }
                }
            }
            let new_equivalent = new_child_map.remove(child_name);
            let old_equivalent = old_child_map.remove(child_name.as_str());
            match (new_equivalent, old_equivalent) {
                (Some(new), Some(old)) => node_queue.push_back((child_node, old, new)),
                (_, None) => anyhow::bail!(
                    "The child '{child_name}' of Instance '{}' would be removed.\n\
                    Syncback cannot add or remove Instances from project {}",
                    old_inst.name(),
                    project_path.display()
                ),
                (None, _) => anyhow::bail!(
                    "The child '{child_name}' of Instance '{}' is present only in a project file,\n\
                    and not the provided file. Syncback cannot add or remove Instances from project:\n{}.",
                    old_inst.name(), project_path.display(),
                )
            }
        }

        // All of the children in this loop are by their nature not in the
        // project, so we just need to run syncback on them.
        for (name, new_child) in new_child_map.drain() {
            let parent_path = match ref_to_path_map.get(&new_child.parent()) {
                Some(path) => path.clone(),
                None => {
                    log::debug!("Skipping child {name} of node because it has no parent_path");
                    continue;
                }
            };

            // If a child also exists in the old tree, it will be caught in the
            // syncback on the project node path above (or is itself a node).
            // So the only things we need to run seperately is new children.
            if old_child_map.remove(name.as_str()).is_none() {
                let parent_middleware =
                    Middleware::middleware_for_path(vfs, &project.sync_rules, &parent_path)?
                        .expect("project nodes should have a middleware if they have children.");
                // Project and model files serialize their descendants themselves.
                // Scheduling a new child again would write a path inside a file.
                if !parent_middleware.handles_own_properties() {
                    descendant_snapshots.push(snapshot.with_base_path(
                        &parent_path,
                        new_child.referent(),
                        None,
                    )?);
                }
            }
        }
        removed_descendants.extend(old_child_map.drain().map(|(_, v)| v));
        node_changed_map.push((&node.properties, &node.attributes, old_inst))
    }
    let mut fs_snapshot = FsSnapshot::new();

    let mut needs_reserialize = removed_stale_properties;
    if !needs_reserialize {
        for (node_properties, node_attributes, old_inst) in node_changed_map {
            if project_node_should_reserialize(node_properties, node_attributes, old_inst)? {
                needs_reserialize = true;
                break;
            }
        }
    }
    if needs_reserialize {
        fs_snapshot.add_file(project_path, serde_json::to_vec_pretty(&project)?);
    }

    Ok(SyncbackReturn {
        fs_snapshot,
        children: descendant_snapshots,
        removed_children: removed_descendants,
    })
}

/// Syncs properties from the new instance into the project node.
/// Returns `true` if any stale properties were removed (i.e. properties
/// that existed in the project node but are now at their engine default).
fn project_node_property_syncback(
    _snapshot: &SyncbackSnapshot,
    filtered_properties: UstrMap<&Variant>,
    new_inst: &Instance,
    node: &mut ProjectNode,
) -> bool {
    let properties = &mut node.properties;
    let mut attributes = BTreeMap::new();
    for (&name, &value) in &filtered_properties {
        match value {
            Variant::Attributes(attrs) => {
                for (attr_name, attr_value) in attrs.iter() {
                    // We (probably) don't want to preserve internal attributes,
                    // only user defined ones.
                    if attr_name.starts_with("RBX") {
                        continue;
                    }
                    attributes.insert(
                        attr_name.clone(),
                        UnresolvedValue::from_variant_unambiguous(attr_value.clone()),
                    );
                }
            }
            _ => {
                properties.insert(
                    name,
                    UnresolvedValue::from_variant(value.clone(), &new_inst.class, &name),
                );
            }
        }
    }

    // Remove stale properties: entries that exist in the project node's
    // $properties but are no longer in the filtered (non-default) properties
    // from the instance. This handles the case where Studio resets a property
    // to its engine default — filter_properties won't include it, so we need
    // to clean up the now-stale project entry.
    let class_data = rbx_reflection_database::get()
        .ok()
        .and_then(|db| db.classes.get(new_inst.class.as_str()));
    let len_before = properties.len();
    properties.retain(|prop_name, _| {
        if filtered_properties.contains_key(prop_name) {
            return true;
        }
        // Only remove if the property has a known default value in the
        // reflection database. If there's no default, the property might be
        // absent from the instance for other reasons (e.g. unknown property),
        // so we conservatively keep it.
        if let Some(data) = &class_data {
            if data.default_properties.contains_key(prop_name.as_str()) {
                log::debug!(
                    "Removing stale property '{}' from project node for class '{}': \
                     value has been reset to engine default",
                    prop_name,
                    new_inst.class
                );
                return false;
            }
        }
        true
    });
    let removed_stale = properties.len() < len_before;

    node.attributes = attributes;
    removed_stale
}

fn project_node_property_syncback_path(
    snapshot: &SyncbackSnapshot,
    new_inst: &Instance,
    node: &mut ProjectNode,
) -> anyhow::Result<bool> {
    if super::rule_defaults::syncback_state(snapshot)?.is_some() {
        let original = node.clone();
        let mut properties = indexmap::IndexMap::new();
        let mut attributes = indexmap::IndexMap::new();
        for (key, value) in snapshot
            .get_path_filtered_properties(new_inst.referent())
            .unwrap()
        {
            match value {
                Variant::Attributes(attrs) => {
                    for (key, value) in attrs.iter().filter(|(key, _)| !key.starts_with("RBX")) {
                        attributes.insert(
                            key.clone(),
                            UnresolvedValue::from_variant_unambiguous(value.clone()),
                        );
                    }
                }
                _ => {
                    properties.insert(
                        key,
                        UnresolvedValue::from_variant(value.clone(), &new_inst.class, &key),
                    );
                }
            }
        }
        let mut mask = node.rule_default_mask().clone();
        super::rule_defaults::rewrite_metadata(
            snapshot,
            &mut properties,
            &mut attributes,
            &mut mask,
            &mut node.ignore_unknown_instances,
        )?;
        node.rule_defaults = (!mask.is_empty()).then(|| Box::new(mask));
        let context = super::lua::project_run_context(snapshot)?;
        if let Some(value) = context {
            properties.insert(
                "RunContext".into(),
                UnresolvedValue::from_variant_unambiguous(value),
            );
        }
        node.properties = properties.into_iter().collect();
        node.attributes = attributes.into_iter().collect();
        return Ok(*node != original);
    }
    let mut filtered_properties = snapshot
        .get_path_filtered_properties(new_inst.referent())
        .unwrap();
    // A native source's default may differ from the engine default stripped by
    // filtering, so keep that override in the existing project property map.
    let context = super::lua::project_run_context(snapshot)?;
    if let Some(value) = &context {
        filtered_properties.insert("RunContext".into(), value);
    }
    Ok(project_node_property_syncback(
        snapshot,
        filtered_properties,
        new_inst,
        node,
    ))
}

fn project_node_property_syncback_no_path(
    snapshot: &SyncbackSnapshot,
    new_inst: &Instance,
    node: &mut ProjectNode,
) -> bool {
    let filtered_properties = filter_properties(snapshot.project(), new_inst);
    project_node_property_syncback(snapshot, filtered_properties, new_inst, node)
}

fn project_node_should_reserialize(
    node_properties: &BTreeMap<Ustr, UnresolvedValue>,
    node_attributes: &BTreeMap<String, UnresolvedValue>,
    instance: InstanceWithMeta,
) -> anyhow::Result<bool> {
    if instance.metadata().rule_defaults.is_some()
        && instance
            .metadata()
            .middleware
            .is_some_and(|m| !m.handles_own_properties())
    {
        // Rule-aware path syncback already compared the complete explicit node,
        // including removals/masks. Shared attributes are not project overrides.
        return Ok(false);
    }
    for (prop_name, unresolved_node_value) in node_properties {
        if let Some(inst_value) = instance.properties().get(prop_name) {
            let node_value = unresolved_node_value
                .clone()
                .resolve(&instance.class_name(), prop_name)?;
            if !variant_eq(inst_value, &node_value) {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }

    match instance.properties().get(&ustr("Attributes")) {
        Some(Variant::Attributes(inst_attributes)) => {
            // This will also catch if one is empty but the other isn't
            if node_attributes.len() != inst_attributes.len() {
                Ok(true)
            } else {
                for (attr_name, unresolved_node_value) in node_attributes {
                    if let Some(inst_value) = inst_attributes.get(attr_name.as_str()) {
                        let node_value = unresolved_node_value.clone().resolve_unambiguous()?;
                        if !variant_eq(inst_value, &node_value) {
                            return Ok(true);
                        }
                    } else {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
        Some(_) => Ok(true),
        None => {
            if !node_attributes.is_empty() {
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }
}

fn infer_class_name(name: &str, parent_class: Option<&str>) -> Option<Ustr> {
    // If className wasn't defined from another source, we may be able
    // to infer one.

    let parent_class = parent_class?;

    if parent_class == "DataModel" {
        // Members of DataModel with names that match known services are
        // probably supposed to be those services.

        let descriptor = rbx_reflection_database::get().unwrap().classes.get(name)?;

        if descriptor.tags.contains(&ClassTag::Service) {
            return Some(ustr(name));
        }
    } else if parent_class == "StarterPlayer" {
        // StarterPlayer has two special members with their own classes.

        if name == "StarterPlayerScripts" || name == "StarterCharacterScripts" {
            return Some(ustr(name));
        }
    } else if parent_class == "Workspace" {
        // Workspace has a special Terrain class inside it
        if name == "Terrain" {
            return Some(ustr(name));
        }
    }

    None
}

// #[cfg(feature = "broken-tests")]
#[cfg(test)]
mod test {
    #[test]
    fn project_path_layers_preserve_model_root_reference_identity() {
        use crate::snapshot::{apply_patch_set, compute_patch_set, RojoTree};
        use rbx_dom_weak::{InstanceBuilder, WeakDom};
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let root = dom.insert(
            dom.root_ref(),
            InstanceBuilder::new("ObjectValue").with_name("StoredRoot"),
        );
        dom.get_by_ref_mut(root)
            .unwrap()
            .properties
            .insert(ustr("Value"), Variant::Ref(root));
        dom.insert(
            root,
            InstanceBuilder::new("ObjectValue")
                .with_name("ToRoot")
                .with_property("Value", root),
        );
        let target = dom.insert(root, InstanceBuilder::new("Part").with_name("Target"));
        dom.insert(
            root,
            InstanceBuilder::new("ObjectValue")
                .with_name("ToSibling")
                .with_property("Value", target),
        );
        let mut binary = Vec::new();
        crate::binary_serialization::to_writer(&mut binary, &dom, &[root]).unwrap();
        let mut xml = Vec::new();
        rbx_xml::to_writer(&mut xml, &dom, &[root], rbx_xml::EncodeOptions::new()).unwrap();
        for (file, bytes) in [("model.rbxm", binary), ("model.rbxmx", xml)] {
            let vfs = Vfs::new(InMemoryFs::new());
            vfs.set_watch_enabled(false);
            vfs.create_dir_all("/project").unwrap();
            vfs.write(format!("/project/{file}"), bytes).unwrap();
            vfs.write(
                "/project/inner.project.json",
                serde_json::json!({"name":"Inner","tree":{"$path":file}}).to_string(),
            )
            .unwrap();
            vfs.write("/project/default.project.json",r#"{"name":"Root","tree":{"$className":"Folder","A":{"$path":"inner.project.json"},"B":{"$path":"inner.project.json"}}}"#).unwrap();
            let mut tree = RojoTree::new(InstanceSnapshot::new());
            let live_root = tree.get_root_id();
            for _ in 0..2 {
                let snapshot = snapshot_from_vfs(
                    &InstanceContext::default(),
                    &vfs,
                    "/project/default.project.json".as_ref(),
                )
                .unwrap();
                let patch = compute_patch_set(snapshot, &tree, live_root);
                apply_patch_set(&mut tree, patch);
                for name in ["A", "B"] {
                    let model = tree
                        .descendants(live_root)
                        .find(|node| node.name() == name)
                        .unwrap();
                    assert_eq!(
                        model.properties().get(&ustr("Value")),
                        Some(&Variant::Ref(model.id())),
                        "{file}/{name} self reference"
                    );
                    let children: std::collections::HashMap<_, _> = model
                        .children()
                        .iter()
                        .map(|id| {
                            let child = tree.get_instance(*id).unwrap();
                            (child.name().to_owned(), *id)
                        })
                        .collect();
                    assert_eq!(
                        tree.get_instance(children["ToRoot"])
                            .unwrap()
                            .properties()
                            .get(&ustr("Value")),
                        Some(&Variant::Ref(model.id())),
                        "{file}/{name} child-to-root reference"
                    );
                    assert_eq!(
                        tree.get_instance(children["ToSibling"])
                            .unwrap()
                            .properties()
                            .get(&ustr("Value")),
                        Some(&Variant::Ref(children["Target"])),
                        "{file}/{name} sibling reference"
                    );
                }
            }
        }
    }
    use super::*;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[test]
    fn identical_project_and_metadata_classes_are_not_conflicts() {
        for class in ["StarterPlayerScripts", "StarterCharacterScripts", "Part"] {
            let mut fs = InMemoryFs::new();
            fs.load_snapshot(
                "/project",
                VfsSnapshot::dir([
                    (
                        "default.project.json",
                        VfsSnapshot::file(format!(
                            r#"{{"name":"Root","tree":{{"$className":"{class}","$path":"src"}}}}"#
                        )),
                    ),
                    (
                        "src",
                        VfsSnapshot::dir([
                            (
                                "init.meta.json",
                                VfsSnapshot::file(format!(
                                    r#"{{"className":"{class}","ignoreUnknownInstances":true}}"#
                                )),
                            ),
                            ("child.luau", VfsSnapshot::file("return 42")),
                        ]),
                    ),
                ]),
            )
            .unwrap();
            let snapshot = snapshot_from_vfs(
                &InstanceContext::new(),
                &Vfs::new(fs),
                Path::new("/project/default.project.json"),
            )
            .unwrap()
            .unwrap();
            assert_eq!(snapshot.class_name.as_str(), class);
            assert!(snapshot.metadata.ignore_unknown_instances);
            assert_eq!(snapshot.children.len(), 1);
            assert_eq!(snapshot.children[0].name, "child");
        }
    }

    #[test]
    fn different_project_and_metadata_classes_still_report_both_paths() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/project",
            VfsSnapshot::dir([
                (
                    "default.project.json",
                    VfsSnapshot::file(
                        r#"{"name":"Root","tree":{"$className":"Part","$path":"src"}}"#,
                    ),
                ),
                (
                    "src",
                    VfsSnapshot::dir([(
                        "init.meta.json",
                        VfsSnapshot::file(r#"{"className":"Model"}"#),
                    )]),
                ),
            ]),
        )
        .unwrap();
        let error = snapshot_from_vfs(
            &InstanceContext::new(),
            &Vfs::new(fs),
            Path::new("/project/default.project.json"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Part") && error.contains("Model"));
        assert!(error.contains("/project/default.project.json") && error.contains("src"));
    }

    #[test]
    fn unknown_descendants_subtree_options_preserve_legacy_node_defaults() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(r#"{"name":"Root","tree":{
                "$className":"Folder",
                "OptIn":{"$path":"src","$ignoreUnknownDescendants":true},
                "Default":{"$path":"src"},
                "Strict":{"$className":"Folder","$ignoreUnknownDescendants":false,"Mapped":{"$path":"src"}},
                "OwnTrue":{"$path":"src","$ignoreUnknownInstances":true}
            }}"#)),
            ("src", VfsSnapshot::dir([("child.luau", VfsSnapshot::file("return 1"))])),
        ])).unwrap();
        let snapshot = snapshot_from_vfs(
            &InstanceContext::new(),
            &Vfs::new(fs),
            Path::new("/project/default.project.json"),
        )
        .unwrap()
        .unwrap();
        let child = |name: &str| {
            snapshot
                .children
                .iter()
                .find(|child| child.name == name)
                .unwrap()
        };
        assert!(snapshot.metadata.ignore_unknown_instances);
        assert!(child("OptIn").children[0].metadata.ignore_unknown_instances);
        assert!(!child("Default").metadata.ignore_unknown_instances);
        assert!(
            !child("Default").children[0]
                .metadata
                .ignore_unknown_instances
        );
        assert!(child("Strict").metadata.ignore_unknown_instances);
        assert!(
            !child("Strict").children[0]
                .metadata
                .ignore_unknown_instances
        );
        assert!(child("OwnTrue").metadata.ignore_unknown_instances);
        assert!(
            !child("OwnTrue").children[0]
                .metadata
                .ignore_unknown_instances
        );
    }

    #[test]
    fn unknown_descendants_reload_does_not_remember_removed_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("default.project.json");
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/child.luau"), "return 1").unwrap();
        let mut context = InstanceContext::new();
        for policy in [Some(true), Some(false), None, Some(true), None] {
            let mut tree = serde_json::json!({"$path":"src"});
            if let Some(policy) = policy {
                tree["$ignoreUnknownDescendants"] = policy.into();
            }
            std::fs::write(
                &project,
                serde_json::to_vec(&serde_json::json!({"name":"Root","tree":tree})).unwrap(),
            )
            .unwrap();
            // Fresh VFS content, but the exact context used by live resnapshots.
            let vfs = Vfs::new(memofs::StdBackend::new().unwrap());
            let snapshot = snapshot_from_vfs(&context, &vfs, &project)
                .unwrap()
                .unwrap();
            assert_eq!(
                snapshot.metadata.ignore_unknown_instances,
                policy == Some(true)
            );
            assert_eq!(
                snapshot.children[0].metadata.ignore_unknown_instances,
                policy == Some(true)
            );
            context = snapshot.metadata.context;
            assert_eq!(context.ignore_unknown_descendants, None);
        }

        context.ignore_unknown_descendants = Some(true);
        for policy in [Some(false), None, Some(true), None] {
            let mut node: ProjectNode = crate::json::from_str(r#"{"$path":"src"}"#).unwrap();
            node.ignore_unknown_descendants = policy;
            let snapshot = snapshot_project_node(
                &context,
                &project,
                "Subtree",
                &node,
                &Vfs::new(memofs::StdBackend::new().unwrap()),
                None,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                snapshot.children[0].metadata.ignore_unknown_instances,
                policy != Some(false)
            );
            context = snapshot.metadata.context;
            assert_eq!(context.ignore_unknown_descendants, Some(true));
        }
    }

    #[test]
    fn unknown_descendants_reach_binary_and_xml_model_children() {
        use rbx_dom_weak::{InstanceBuilder, WeakDom};
        let dom = WeakDom::new(
            InstanceBuilder::new("DataModel").with_child(
                InstanceBuilder::new("Folder")
                    .with_child(InstanceBuilder::new("Folder").with_name("Embedded")),
            ),
        );
        let mut binary = Vec::new();
        crate::binary_serialization::to_writer(&mut binary, &dom, dom.root().children()).unwrap();
        let mut xml = Vec::new();
        rbx_xml::to_writer(
            &mut xml,
            &dom,
            dom.root().children(),
            rbx_xml::EncodeOptions::new(),
        )
        .unwrap();
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/models",
            VfsSnapshot::dir([
                ("binary.rbxm", VfsSnapshot::file(binary)),
                ("xml.rbxmx", VfsSnapshot::file(xml)),
            ]),
        )
        .unwrap();
        let vfs = Vfs::new(fs);
        let mut context = InstanceContext::new();
        for policy in [None, Some(true), Some(false)] {
            context.ignore_unknown_descendants = policy;
            for path in ["/models/binary.rbxm", "/models/xml.rbxmx"] {
                let snapshot = snapshot_from_vfs(&context, &vfs, Path::new(path))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    snapshot.metadata.ignore_unknown_instances,
                    policy == Some(true)
                );
                assert_eq!(
                    snapshot.children[0].metadata.ignore_unknown_instances,
                    policy == Some(true)
                );
            }
        }
    }

    #[test]
    fn unknown_descendants_inherit_without_overriding_explicit_metadata() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(r#"{"name":"Root","tree":{
                "$path":"src","$ignoreUnknownDescendants":true,
                "Strict":{"$path":"strict","$ignoreUnknownDescendants":false},
                "OwnFalse":{"$path":"own","$ignoreUnknownInstances":false},
                "Nested":{"$path":"nested.project.json"}
            }}"#)),
            ("src", VfsSnapshot::dir([
                ("child.luau", VfsSnapshot::file("return 1")),
                ("child.meta.json", VfsSnapshot::file(r#"{"ignoreUnknownInstances":false}"#)),
                ("model.model.json", VfsSnapshot::file(r#"{"className":"Folder","children":[{"name":"Embedded","className":"Folder"}]}"#)),
                ("directory", VfsSnapshot::dir([
                    ("init.meta.json", VfsSnapshot::file(r#"{"ignoreUnknownInstances":false}"#)),
                    ("grandchild.luau", VfsSnapshot::file("return 2")),
                ])),
            ])),
            ("strict", VfsSnapshot::dir([("child.luau", VfsSnapshot::file("return 3"))])),
            ("own", VfsSnapshot::dir([("child.luau", VfsSnapshot::file("return 4"))])),
            ("nested.project.json", VfsSnapshot::file(r#"{"name":"Nested","tree":{"$path":"strict"}}"#)),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        let snapshot = snapshot_from_vfs(
            &InstanceContext::new(),
            &vfs,
            Path::new("/project/default.project.json"),
        )
        .unwrap()
        .unwrap();
        let child = |name: &str| {
            snapshot
                .children
                .iter()
                .find(|child| child.name == name)
                .unwrap()
        };
        assert!(snapshot.metadata.ignore_unknown_instances);
        assert!(!child("child").metadata.ignore_unknown_instances);
        assert!(!child("directory").metadata.ignore_unknown_instances);
        assert!(
            child("directory").children[0]
                .metadata
                .ignore_unknown_instances
        );
        assert!(child("model").metadata.ignore_unknown_instances);
        assert!(child("model").children[0].metadata.ignore_unknown_instances);
        assert!(!child("Strict").metadata.ignore_unknown_instances);
        assert!(
            !child("Strict").children[0]
                .metadata
                .ignore_unknown_instances
        );
        assert!(!child("OwnFalse").metadata.ignore_unknown_instances);
        assert!(
            child("OwnFalse").children[0]
                .metadata
                .ignore_unknown_instances
        );
        assert!(
            child("Nested").children[0]
                .metadata
                .ignore_unknown_instances
        );
    }

    #[test]
    fn indirect_project_cycles_report_the_chain_and_clear_traversal_state() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.project.json");
        let second = dir.path().join("second.project.json");
        std::fs::write(
            &first,
            r#"{"name":"First","tree":{"$path":"second.project.json"}}"#,
        )
        .unwrap();
        std::fs::write(
            &second,
            r#"{"name":"Second","tree":{"$path":"./first.project.json"}}"#,
        )
        .unwrap();
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let context = InstanceContext::default();
        let error = snapshot_from_vfs(&context, &vfs, &first).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Circular project reference"), "{message}");
        assert!(message.contains("first.project.json"), "{message}");
        assert!(message.contains("second.project.json"), "{message}");
        vfs.write(
            &second,
            r#"{"name":"Second","tree":{"$className":"Folder"}}"#,
        )
        .unwrap();
        for _ in 0..2 {
            let snapshot = snapshot_from_vfs(&context, &vfs, &first).unwrap().unwrap();
            assert_eq!(snapshot.class_name, "Folder");
        }
    }

    #[test]
    fn independent_project_aliases_are_not_cycles() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("root.project.json", VfsSnapshot::file(r#"{"name":"Root","tree":{"$className":"Folder","First":{"$path":"leaf.project.json"},"Second":{"$path":"leaf.project.json"}}}"#)),
            ("leaf.project.json", VfsSnapshot::file(r#"{"name":"Leaf","tree":{"$className":"Folder"}}"#)),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        let snapshot = snapshot_from_vfs(
            &InstanceContext::default(),
            &vfs,
            Path::new("/project/root.project.json"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.children.len(), 2);
        assert_ne!(snapshot.children[0].name, snapshot.children[1].name);
    }

    #[ignore = "Functionality moved to root snapshot middleware"]
    #[test]
    fn project_from_folder() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([(
                "default.project.json",
                VfsSnapshot::file(
                    r#"
                    {
                        "name": "indirect-project",
                        "tree": {
                            "$className": "Folder"
                        }
                    }
                "#,
                ),
            )]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_from_direct_file() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([(
                "hello.project.json",
                VfsSnapshot::file(
                    r#"
                    {
                        "name": "direct-project",
                        "tree": {
                            "$className": "Model"
                        }
                    }
                "#,
                ),
            )]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/hello.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_resolved_properties() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.project.json",
            VfsSnapshot::file(
                r#"
                    {
                        "name": "resolved-properties",
                        "tree": {
                            "$className": "StringValue",
                            "$properties": {
                                "Value": {
                                    "String": "Hello, world!"
                                }
                            }
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_unresolved_properties() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.project.json",
            VfsSnapshot::file(
                r#"
                    {
                        "name": "unresolved-properties",
                        "tree": {
                            "$className": "StringValue",
                            "$properties": {
                                "Value": "Hi!"
                            }
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_children() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.project.json",
            VfsSnapshot::file(
                r#"
                    {
                        "name": "children",
                        "tree": {
                            "$className": "Folder",

                            "Child": {
                                "$className": "Model"
                            }
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_path_to_txt() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-project",
                        "tree": {
                            "$path": "other.txt"
                        }
                    }
                "#,
                    ),
                ),
                ("other.txt", VfsSnapshot::file("Hello, world!")),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_path_to_project() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-project",
                        "tree": {
                            "$path": "other.project.json"
                        }
                    }
                "#,
                    ),
                ),
                (
                    "other.project.json",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "other-project",
                        "tree": {
                            "$className": "Model"
                        }
                    }
                "#,
                    ),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_path_to_project_with_children() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-child-project",
                        "tree": {
                            "$path": "other.project.json"
                        }
                    }
                "#,
                    ),
                ),
                (
                    "other.project.json",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "other-project",
                        "tree": {
                            "$className": "Folder",

                            "SomeChild": {
                                "$className": "Model"
                            }
                        }
                    }
                "#,
                    ),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    /// Ensures that if a property is defined both in the resulting instance
    /// from $path and also in $properties, that the $properties value takes
    /// precedence.
    #[test]
    fn project_path_property_overrides() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-property-override",
                        "tree": {
                            "$path": "other.project.json",
                            "$properties": {
                                "Value": "Changed"
                            }
                        }
                    }
                "#,
                    ),
                ),
                (
                    "other.project.json",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "other-project",
                        "tree": {
                            "$className": "StringValue",
                            "$properties": {
                                "Value": "Original"
                            }
                        }
                    }
                "#,
                    ),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn no_name_project() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([(
                "default.project.json",
                VfsSnapshot::file(
                    r#"
                    {
                        "tree": {
                            "$className": "Model"
                        }
                    }
                "#,
                ),
            )]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json"),
            "no_name_project",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }
}
