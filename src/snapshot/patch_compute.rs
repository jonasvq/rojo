//! Defines the algorithm for computing a roughly-minimal patch set given an
//! existing instance tree and an instance snapshot.

use std::{
    collections::{HashMap, VecDeque},
    mem::take,
};

use rbx_dom_weak::{
    types::{Ref, Variant},
    ustr, HashMapExt as _, UstrMap, UstrSet,
};

use crate::{variant_eq::variant_eq, RojoRef, REF_POINTER_ATTRIBUTE_PREFIX};

use super::{
    patch::{PatchAdd, PatchSet, PatchUpdate},
    InstanceSnapshot, InstanceWithMeta, RojoTree,
};

#[profiling::function]
pub fn compute_patch_set(snapshot: Option<InstanceSnapshot>, tree: &RojoTree, id: Ref) -> PatchSet {
    let mut patch_set = PatchSet::new();

    if let Some(snapshot) = snapshot {
        let mut context = ComputePatchContext::default();

        compute_patch_set_internal(&mut context, snapshot, tree, id, &mut patch_set);

        // Rewrite Ref properties to refer to instance IDs instead of snapshot IDs
        // for all of the IDs that we know about so far.
        rewrite_refs_in_updates(&context, &mut patch_set.updated_instances);
        rewrite_refs_in_additions(&context, &mut patch_set.added_instances);
    } else if id != tree.get_root_id() {
        patch_set.removed_instances.push(id);
    }

    patch_set
}

#[derive(Default)]
struct ComputePatchContext {
    snapshot_id_to_instance_id: HashMap<Ref, Ref>,
    #[cfg(test)]
    child_candidates_examined: usize,
}

fn rewrite_refs_in_updates(context: &ComputePatchContext, updates: &mut [PatchUpdate]) {
    for update in updates {
        for property_value in update.changed_properties.values_mut() {
            if let Some(Variant::Ref(referent)) = property_value {
                if let Some(&instance_ref) = context.snapshot_id_to_instance_id.get(referent) {
                    *property_value = Some(Variant::Ref(instance_ref));
                }
            }
        }
    }
}

fn rewrite_refs_in_additions(context: &ComputePatchContext, additions: &mut [PatchAdd]) {
    for addition in additions {
        rewrite_refs_in_snapshot(context, &mut addition.instance);
    }
}

fn rewrite_refs_in_snapshot(context: &ComputePatchContext, snapshot: &mut InstanceSnapshot) {
    for property_value in snapshot.properties.values_mut() {
        if let Variant::Ref(referent) = property_value {
            if let Some(&instance_referent) = context.snapshot_id_to_instance_id.get(referent) {
                *property_value = Variant::Ref(instance_referent);
            }
        }
    }

    for child in &mut snapshot.children {
        rewrite_refs_in_snapshot(context, child);
    }
}

fn compute_patch_set_internal(
    context: &mut ComputePatchContext,
    mut snapshot: InstanceSnapshot,
    tree: &RojoTree,
    id: Ref,
    patch_set: &mut PatchSet,
) {
    if snapshot.snapshot_id.is_some() {
        context
            .snapshot_id_to_instance_id
            .insert(snapshot.snapshot_id, id);
    }

    let instance = tree
        .get_instance(id)
        .expect("Instance did not exist in tree");

    compute_property_patches(&mut snapshot, &instance, patch_set, tree);
    compute_children_patches(context, &mut snapshot, tree, id, patch_set);
}

fn compute_property_patches(
    snapshot: &mut InstanceSnapshot,
    instance: &InstanceWithMeta,
    patch_set: &mut PatchSet,
    tree: &RojoTree,
) {
    if let Some(Variant::Attributes(attributes)) = snapshot.properties.get(&ustr("Attributes")) {
        if let Some(id) = super::patch_apply::specified_id_from_attributes(attributes) {
            // Patch application derives this metadata from Rojo_Id too. Compare
            // the effective ID rather than repeatedly clearing and restoring it.
            snapshot.metadata.specified_id = Some(id);
        }
    }
    let mut visited_properties = UstrSet::default();
    let mut changed_properties = UstrMap::new();

    let attribute_ref_properties = compute_ref_properties(snapshot, tree);

    let changed_name = if snapshot.name == instance.name() {
        None
    } else {
        Some(take(&mut snapshot.name).into_owned())
    };

    let changed_class_name = if snapshot.class_name == instance.class_name() {
        None
    } else {
        Some(take(&mut snapshot.class_name))
    };

    let changed_metadata = if &snapshot.metadata == instance.metadata() {
        None
    } else {
        Some(take(&mut snapshot.metadata))
    };

    for (name, snapshot_value) in take(&mut snapshot.properties) {
        visited_properties.insert(name);

        match instance.properties().get(&name) {
            Some(instance_value) => {
                if !variant_eq(&snapshot_value, instance_value) {
                    changed_properties.insert(name, Some(snapshot_value));
                }
            }
            None => {
                changed_properties.insert(name, Some(snapshot_value));
            }
        }
    }

    for name in instance.properties().keys() {
        if visited_properties.contains(name) {
            continue;
        }

        changed_properties.insert(*name, None);
    }

    for (name, ref_value) in attribute_ref_properties {
        match (&ref_value, instance.properties().get(&name)) {
            (Some(referent), Some(instance_value)) => {
                if referent != instance_value {
                    changed_properties.insert(name, ref_value);
                } else {
                    changed_properties.remove(&name);
                }
            }
            (Some(_), None) | (None, Some(_)) => {
                changed_properties.insert(name, ref_value);
            }
            (None, None) => {
                changed_properties.remove(&name);
            }
        }
    }

    // !!!!!!!!!! UGLY HACK !!!!!!!!!!
    //
    // See RojoTree::insert_instance. Adjust that code also if you are touching this.
    let actual_class = changed_class_name.unwrap_or(instance.class_name());
    match actual_class.as_str() {
        "Model" | "Actor" | "Tool" | "HopperBin" | "Flag" | "WorldModel" | "Workspace"
        | "Status" => {
            let migration_prop = ustr("NeedsPivotMigration");
            // We want to just ignore this if it's being removed by a patch.
            // Normally this would not matter because serving != building but
            // if we start syncing models using SerializationService
            // (or GetObjects) it will affect how Studio deserializes things.
            if !instance.properties().contains_key(&migration_prop) {
                changed_properties.insert(migration_prop, Some(Variant::Bool(false)));
            }
            match changed_properties.get(&migration_prop) {
                Some(Some(Variant::Bool(_))) => {}
                Some(None) => {
                    changed_properties.remove(&migration_prop);
                }
                _ => {
                    changed_properties.insert(migration_prop, Some(Variant::Bool(false)));
                }
            }
        }
        _ => {}
    };

    if changed_properties.is_empty()
        && changed_name.is_none()
        && changed_class_name.is_none()
        && changed_metadata.is_none()
    {
        return;
    }

    patch_set.updated_instances.push(PatchUpdate {
        id: instance.id(),
        changed_name,
        changed_class_name,
        changed_properties,
        changed_metadata,
    });
}

fn compute_children_patches(
    context: &mut ComputePatchContext,
    snapshot: &mut InstanceSnapshot,
    tree: &RojoTree,
    id: Ref,
    patch_set: &mut PatchSet,
) {
    let instance = tree
        .get_instance(id)
        .expect("Instance did not exist in tree");

    let instance_children = instance.children();

    let mut paired_instances = vec![false; instance_children.len()];

    // Keep sibling order within each name/class group, without rescanning the
    // already paired prefix for every child in a large model.
    let mut candidates_by_name_class: HashMap<_, HashMap<_, VecDeque<usize>>> = HashMap::new();
    for (index, child_id) in instance_children.iter().enumerate() {
        let child = tree
            .inner()
            .get_by_ref(*child_id)
            .expect("Instance did not exist in tree");
        candidates_by_name_class
            .entry(child.name.as_str())
            .or_default()
            .entry(child.class)
            .or_default()
            .push_back(index);
    }

    for snapshot_child in take(&mut snapshot.children) {
        let matching_index = candidates_by_name_class
            .get_mut(snapshot_child.name.as_ref())
            .and_then(|by_class| by_class.get_mut(&snapshot_child.class_name))
            .and_then(|candidates| {
                while let Some(index) = candidates.pop_front() {
                    #[cfg(test)]
                    {
                        context.child_candidates_examined += 1;
                    }
                    if !paired_instances[index] {
                        return Some(index);
                    }
                }
                None
            })
            .or_else(|| {
                // Global emitLegacyScripts changes re-snapshot ancestors.
                // Keep the identity of the same script source so clients
                // receive a class update and can preserve unmanaged children.
                let source = snapshot_child.metadata.instigating_source.as_ref()?;
                let other_class = match snapshot_child.class_name.as_str() {
                    "Script" => ustr("LocalScript"),
                    "LocalScript" => ustr("Script"),
                    _ => return None,
                };
                let bucket = candidates_by_name_class
                    .get(snapshot_child.name.as_ref())?
                    .get(&other_class)?;
                let mut candidates = bucket.iter().copied().filter(|index| {
                    #[cfg(test)]
                    {
                        context.child_candidates_examined += 1;
                    }
                    !paired_instances[*index]
                        && tree
                            .get_instance(instance_children[*index])
                            .expect("Instance did not exist in tree")
                            .metadata()
                            .instigating_source
                            .as_ref()
                            == Some(source)
                });
                let candidate = candidates.next()?;
                if candidates.next().is_some() {
                    return None;
                }
                Some(candidate)
            });

        match matching_index {
            Some(index) => {
                paired_instances[index] = true;
                compute_patch_set_internal(
                    context,
                    snapshot_child,
                    tree,
                    instance_children[index],
                    patch_set,
                );
            }
            None => {
                patch_set.added_instances.push(PatchAdd {
                    parent_id: id,
                    instance: snapshot_child,
                });
            }
        }
    }

    for (instance_index, instance_child_id) in instance_children.iter().enumerate() {
        if paired_instances[instance_index] {
            continue;
        }

        patch_set.removed_instances.push(*instance_child_id);
    }
}

fn compute_ref_properties(
    snapshot: &InstanceSnapshot,
    tree: &RojoTree,
) -> UstrMap<Option<Variant>> {
    let mut map = UstrMap::new();
    let attributes = match snapshot.properties.get(&ustr("Attributes")) {
        Some(Variant::Attributes(attrs)) => attrs,
        _ => return map,
    };

    for (attr_name, attr_value) in attributes.iter() {
        let prop_name = match attr_name.strip_prefix(REF_POINTER_ATTRIBUTE_PREFIX) {
            Some(str) => str,
            None => continue,
        };
        let rojo_ref = match attr_value {
            Variant::String(str) => RojoRef::new(str.clone()),
            Variant::BinaryString(bytes) => {
                if let Ok(str) = std::str::from_utf8(bytes.as_ref()) {
                    RojoRef::new(str.to_string())
                } else {
                    log::warn!(
                        "IDs specified by referent property attributes must be valid UTF-8 strings"
                    );
                    continue;
                }
            }
            _ => {
                log::warn!(
                    "Attribute {attr_name} is of type {:?} when it was \
                expected to be a String",
                    attr_value.ty()
                );
                continue;
            }
        };
        if let Some(target_id) = tree.get_specified_id(&rojo_ref) {
            map.insert(ustr(prop_name), Some(Variant::Ref(target_id)));
        } else {
            map.insert(ustr(prop_name), None);
        }
    }

    map
}

#[cfg(test)]
mod test {
    use super::*;

    use std::borrow::Cow;

    #[test]
    fn wide_sibling_matching_examines_a_linear_number_of_candidates() {
        let count = 1024;
        let snapshot = || {
            InstanceSnapshot::new().children(
                (0..count)
                    .map(|index| InstanceSnapshot::new().name(format!("Child{index}")))
                    .collect::<Vec<_>>(),
            )
        };
        let tree = RojoTree::new(snapshot());
        let mut context = ComputePatchContext::default();
        let mut patch = PatchSet::new();
        compute_patch_set_internal(
            &mut context,
            snapshot(),
            &tree,
            tree.get_root_id(),
            &mut patch,
        );
        assert!(patch.added_instances.is_empty());
        assert!(patch.removed_instances.is_empty());
        assert!(patch.updated_instances.is_empty());
        assert!(
            context.child_candidates_examined <= count * 2,
            "matching {count} unchanged siblings examined {} candidates",
            context.child_candidates_examined
        );
    }

    #[test]
    fn duplicate_siblings_keep_first_unpaired_name_and_class_matching() {
        let child = |class, value: &str| {
            InstanceSnapshot::new()
                .name("Duplicate")
                .class_name(class)
                .properties(UstrMap::from_iter([(ustr("Value"), value.into())]))
        };
        let tree = RojoTree::new(InstanceSnapshot::new().children(vec![
            child("StringValue", "first"),
            child("IntValue", "other class"),
            child("StringValue", "second"),
            child("StringValue", "removed"),
        ]));
        let ids = tree.root().children().to_vec();
        let patch = compute_patch_set(
            Some(InstanceSnapshot::new().children(vec![
                child("IntValue", "changed other class"),
                child("StringValue", "changed first"),
                child("StringValue", "changed second"),
            ])),
            &tree,
            tree.get_root_id(),
        );
        assert!(patch.added_instances.is_empty());
        assert_eq!(patch.removed_instances, vec![ids[3]]);
        let matched: Vec<_> = patch
            .updated_instances
            .iter()
            .map(|update| update.id)
            .collect();
        assert_eq!(matched, vec![ids[1], ids[0], ids[2]]);
    }

    #[test]
    fn script_class_fallback_does_not_reuse_unrelated_or_ambiguous_sources() {
        use super::super::InstanceMetadata;
        let child = |class, source: &str| {
            InstanceSnapshot::new()
                .name("Client")
                .class_name(class)
                .metadata(InstanceMetadata::new().instigating_source(std::path::Path::new(source)))
        };
        for children in [
            vec![child("LocalScript", "/other")],
            vec![child("LocalScript", "/same"), child("LocalScript", "/same")],
        ] {
            let count = children.len();
            let tree = RojoTree::new(InstanceSnapshot::new().children(children));
            let snapshot = InstanceSnapshot::new().children(vec![child("Script", "/same")]);
            let patch = compute_patch_set(Some(snapshot), &tree, tree.get_root_id());
            assert_eq!(patch.removed_instances.len(), count);
            assert_eq!(patch.added_instances.len(), 1);
        }
    }

    #[test]
    fn toggling_legacy_scripts_preserves_child_id_in_full_project_snapshot() {
        use crate::snapshot::{apply_patch_set, InstanceContext};
        use crate::snapshot_middleware::snapshot_from_vfs;
        use memofs::{InMemoryFs, Vfs, VfsSnapshot};
        let mut fs = InMemoryFs::new();
        let project = |legacy| {
            format!(r#"{{"name":"Root","emitLegacyScripts":{legacy},"tree":{{"$path":"src"}}}}"#)
        };
        fs.load_snapshot(
            "/project",
            VfsSnapshot::dir([
                ("default.project.json", VfsSnapshot::file(project(true))),
                (
                    "src",
                    VfsSnapshot::dir([(
                        "Client",
                        VfsSnapshot::dir([
                            ("init.client.lua", VfsSnapshot::file("print('client')")),
                            (
                                "init.meta.json",
                                VfsSnapshot::file(r#"{"ignoreUnknownInstances":true}"#),
                            ),
                        ]),
                    )]),
                ),
            ]),
        )
        .unwrap();
        let vfs = Vfs::new(fs);
        let path = std::path::Path::new("/project/default.project.json");
        let initial = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let mut tree = RojoTree::new(initial);
        let root = tree.get_root_id();
        let client = tree.root().children()[0];
        assert_eq!(
            tree.get_instance(client).unwrap().class_name(),
            "LocalScript"
        );
        for legacy in [false, true] {
            vfs.write(path, project(legacy)).unwrap();
            let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                .unwrap()
                .unwrap();
            let patch = compute_patch_set(Some(snapshot), &tree, root);
            assert!(
                patch.removed_instances.is_empty(),
                "A class toggle must not remove the managed identity"
            );
            assert!(
                patch.added_instances.is_empty(),
                "A class toggle must be an update"
            );
            assert!(patch
                .updated_instances
                .iter()
                .any(|update| update.id == client
                    && update.changed_class_name.as_deref()
                        == Some(if legacy { "LocalScript" } else { "Script" })));
            apply_patch_set(&mut tree, patch);
            assert!(
                tree.get_instance(client)
                    .unwrap()
                    .metadata()
                    .ignore_unknown_instances
            );
        }
    }

    /// This test makes sure that rewriting refs in instance update patches to
    /// instances that already exists works. We should be able to correlate the
    /// snapshot ID and instance ID during patch computation and replace the
    /// value before returning from compute_patch_set.
    #[test]
    fn rewrite_ref_existing_instance_update() {
        let tree = RojoTree::new(InstanceSnapshot::new().name("foo").class_name("foo"));

        let root_id = tree.get_root_id();

        // This snapshot should be identical to the existing tree except for the
        // addition of a prop named Self, which is a self-referential Ref.
        let snapshot_id = Ref::new();
        let snapshot = InstanceSnapshot {
            snapshot_id,
            properties: UstrMap::from_iter([(ustr("Self"), Variant::Ref(snapshot_id))]),

            metadata: Default::default(),
            name: Cow::Borrowed("foo"),
            class_name: ustr("foo"),
            children: Vec::new(),
        };

        let patch_set = compute_patch_set(Some(snapshot), &tree, root_id);

        let expected_patch_set = PatchSet {
            updated_instances: vec![PatchUpdate {
                id: root_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: UstrMap::from_iter([(
                    ustr("Self"),
                    Some(Variant::Ref(root_id)),
                )]),
                changed_metadata: None,
            }],
            added_instances: Vec::new(),
            removed_instances: Vec::new(),
        };

        assert_eq!(patch_set, expected_patch_set);
    }

    /// The same as rewrite_ref_existing_instance_update, except that the
    /// property is added in a new instance instead of modifying an existing
    /// one.
    #[test]
    fn rewrite_ref_existing_instance_addition() {
        let tree = RojoTree::new(InstanceSnapshot::new().name("foo").class_name("foo"));

        let root_id = tree.get_root_id();

        // This patch describes the existing instance with a new child added.
        let snapshot_id = Ref::new();
        let snapshot = InstanceSnapshot {
            snapshot_id,
            children: vec![InstanceSnapshot {
                properties: UstrMap::from_iter([(ustr("Self"), Variant::Ref(snapshot_id))]),

                snapshot_id: Ref::none(),
                metadata: Default::default(),
                name: Cow::Borrowed("child"),
                class_name: ustr("child"),
                children: Vec::new(),
            }],

            metadata: Default::default(),
            properties: UstrMap::new(),
            name: Cow::Borrowed("foo"),
            class_name: ustr("foo"),
        };

        let patch_set = compute_patch_set(Some(snapshot), &tree, root_id);

        let expected_patch_set = PatchSet {
            added_instances: vec![PatchAdd {
                parent_id: root_id,
                instance: InstanceSnapshot {
                    snapshot_id: Ref::none(),
                    metadata: Default::default(),
                    properties: UstrMap::from_iter([(ustr("Self"), Variant::Ref(root_id))]),
                    name: Cow::Borrowed("child"),
                    class_name: ustr("child"),
                    children: Vec::new(),
                },
            }],
            updated_instances: Vec::new(),
            removed_instances: Vec::new(),
        };

        assert_eq!(patch_set, expected_patch_set);
    }
}
