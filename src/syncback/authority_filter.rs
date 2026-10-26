//! Apply class source-of-truth policies to explicit syncback input. This does
//! not grant live writes or introduce another filesystem persistence mechanism.
use std::collections::{HashMap, HashSet};

use anyhow::{ensure, Context};
use rbx_dom_weak::{
    types::{ContentType, Ref, Variant},
    Ustr, WeakDom,
};

use super::class_filter::{pairs, Protection};
use crate::{snapshot::RojoTree, PropertyAuthority, StructureAuthority};

pub(super) struct Projection {
    pub tree: WeakDom,
    pub protection: Option<Protection>,
    pub protected_old: HashSet<Ref>,
    pub preserve_native_files: HashSet<Ref>,
    pub required_structure: HashMap<Ref, String>,
}

pub(super) fn apply(old: &RojoTree, mut new: WeakDom) -> anyhow::Result<Projection> {
    if !old.descendants(old.get_root_id()).any(|node| {
        node.metadata()
            .effective_sync_authority
            .as_ref()
            .is_some_and(|authority| !authority.policies().is_empty())
    }) {
        return Ok(Projection {
            tree: new,
            protection: None,
            protected_old: HashSet::new(),
            preserve_native_files: HashSet::new(),
            required_structure: HashMap::new(),
        });
    }
    let mut policies = HashMap::new();
    for node in old.descendants(old.get_root_id()) {
        let policy = node
            .metadata()
            .effective_sync_authority
            .as_ref()
            .map(|authority| authority.resolve(node.class_name().as_str()))
            .transpose()?
            .flatten()
            .cloned();
        policies.insert(node.id(), policy);
    }
    let mut flags = HashMap::new();
    let mut structure_flags = HashMap::new();
    for id in super::descendants(old.inner(), old.get_root_id())
        .into_iter()
        .rev()
    {
        let node = old.get_instance(id).unwrap();
        let filesystem_structure = policies[&id]
            .as_ref()
            .is_some_and(|policy| policy.structure == StructureAuthority::Filesystem);
        structure_flags.insert(
            id,
            filesystem_structure || node.children().iter().any(|child| structure_flags[child]),
        );
        flags.insert(
            id,
            filesystem_structure
                || policies[&id]
                    .as_ref()
                    .is_some_and(|policy| policy.properties == PropertyAuthority::Filesystem)
                || node.children().iter().any(|child| flags[child]),
        );
    }
    let new_flags: HashMap<_, _> = super::descendants(&new, new.root_ref())
        .into_iter()
        .map(|id| (id, false))
        .collect();
    let mut work = vec![(Some(old.get_root_id()), Some(new.root_ref()), None)];
    let mut mapping = HashMap::new();
    let mut copy = Vec::new();
    let mut required_structure = HashMap::new();
    while let Some((old_id, new_id, inherited)) = work.pop() {
        let old_node = old_id.and_then(|id| old.get_instance(id));
        let authority = if let Some(node) = &old_node {
            node.metadata().effective_sync_authority.clone()
        } else {
            inherited
        };
        let Some(new_id) = new_id else {
            ensure!(
                !old_id.is_some_and(|id| structure_flags[&id]),
                "syncAuthority filesystem structure is missing near {}",
                old.inner().full_path_of(old_id.unwrap(), "/")
            );
            continue;
        };
        let node = new
            .get_by_ref(new_id)
            .context("Syncback input instance disappeared")?;
        let policy = if let Some(id) = old_id {
            policies[&id].clone()
        } else {
            authority
                .as_ref()
                .map(|authority| authority.resolve(node.class.as_str()))
                .transpose()?
                .flatten()
                .cloned()
        };
        let input_policy = authority
            .as_ref()
            .map(|authority| authority.resolve(node.class.as_str()))
            .transpose()?
            .flatten();
        if policy
            .as_ref()
            .is_some_and(|policy| policy.structure == StructureAuthority::Filesystem)
            || input_policy.is_some_and(|policy| policy.structure == StructureAuthority::Filesystem)
        {
            let old_node = old_node
                .as_ref()
                .context("syncAuthority filesystem structure cannot import a new instance")?;
            ensure!(
                old_node.name() == node.name && old_node.class_name() == node.class,
                "syncAuthority filesystem structure changed at {}",
                new.full_path_of(new_id, "/")
            );
            required_structure.insert(new_id, new.full_path_of(new_id, "/"));
        }
        if let Some(old_node) = &old_node {
            ensure!(
                mapping.insert(old_node.id(), new_id).is_none(),
                "syncAuthority matched a disk instance twice"
            );
        }
        if policy
            .as_ref()
            .is_some_and(|policy| policy.properties == PropertyAuthority::Filesystem)
        {
            let old_node = old_node
                .as_ref()
                .context("Filesystem properties have no disk baseline for imported instance")?;
            ensure!(
                old_node.class_name() == node.class,
                "Filesystem property class changed at {}",
                new.full_path_of(new_id, "/")
            );
            copy.push((old_node.id(), new_id));
        }
        for (old_child, new_child) in pairs(
            old.inner(),
            &new,
            old_id,
            Some(new_id),
            &flags,
            &new_flags,
            ("syncAuthority", false),
        )? {
            work.push((old_child, new_child, authority.clone()));
        }
    }
    let mut attributes = HashMap::new();
    let mut protected_old = HashSet::new();
    let mut preserve_native_files = HashSet::new();
    for (old_id, new_id) in copy {
        let mut properties = old.get_instance(old_id).unwrap().properties().clone();
        for (name, value) in &mut properties {
            remap(value, &mapping, 0).with_context(|| {
                format!(
                    "Filesystem property {}.{name}",
                    old.inner().full_path_of(old_id, "/")
                )
            })?;
        }
        attributes.insert(new_id, properties.get(&Ustr::from("Attributes")).cloned());
        new.get_by_ref_mut(new_id).unwrap().properties = properties;
        protected_old.insert(old_id);
        if policies[&old_id]
            .as_ref()
            .is_some_and(|policy| policy.structure == StructureAuthority::Filesystem)
        {
            preserve_native_files.insert(new_id);
        }
    }
    Ok(Projection {
        tree: new,
        protection: Some(Protection::new(attributes, "syncAuthority")),
        protected_old,
        preserve_native_files,
        required_structure,
    })
}

fn remap(value: &mut Variant, mapping: &HashMap<Ref, Ref>, depth: usize) -> anyhow::Result<()> {
    ensure!(
        depth <= 128,
        "Filesystem property reference nesting exceeds syncback limit"
    );
    let target = match value {
        Variant::Ref(target) => Some(target),
        Variant::Content(content) => match content.value_mut() {
            ContentType::Object(target) => Some(target),
            _ => None,
        },
        Variant::Attributes(attributes) => {
            let mut mapped = attributes.clone();
            for (name, mut value) in mapped.drain().collect::<Vec<_>>() {
                remap(&mut value, mapping, depth + 1)?;
                attributes.insert(name, value);
            }
            None
        }
        _ => None,
    };
    if let Some(target) = target.filter(|target| target.is_some()) {
        *target = *mapping.get(target).context("syncAuthority cannot preserve a filesystem reference whose target was removed or ambiguous")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use memofs::{InMemoryFs, Vfs};
    use rbx_dom_weak::{
        types::{Attributes, Content},
        InstanceBuilder,
    };
    use serde_json::json;

    fn fixture(project: serde_json::Value) -> RojoTree {
        let vfs = Vfs::new(InMemoryFs::new());
        vfs.set_watch_enabled(false);
        vfs.create_dir_all("/project").unwrap();
        vfs.write("/project/default.project.json", project.to_string())
            .unwrap();
        let project =
            crate::Project::load_exact(&vfs, "/project/default.project.json".as_ref(), None)
                .unwrap();
        crate::build_project(&vfs, &project).unwrap()
    }

    fn named(dom: &WeakDom, name: &str) -> Ref {
        super::super::descendants(dom, dom.root_ref())
            .into_iter()
            .find(|id| dom.get_by_ref(*id).unwrap().name == name)
            .unwrap()
    }

    #[test]
    fn protected_properties_are_per_instance_not_a_subtree_exclusion() {
        let old = fixture(
            json!({"name":"Root","syncAuthority":[{"className":"Folder","properties":"filesystem"},{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","$properties":{"Archivable":false},"Child":{"$className":"Part","$properties":{"Transparency":0.1}}}}),
        );
        let new = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_property("Archivable", true)
                .with_property(
                    "Tags",
                    rbx_dom_weak::types::Tags::from(vec!["changed".into()]),
                )
                .with_child(
                    InstanceBuilder::new("Part")
                        .with_name("Child")
                        .with_property("Transparency", 0.8_f32),
                ),
        );
        let projection = apply(&old, new).unwrap();
        assert_eq!(
            projection.tree.root().properties[&"Archivable".into()],
            Variant::Bool(false)
        );
        assert!(!projection
            .tree
            .root()
            .properties
            .contains_key(&"Tags".into()));
        assert_eq!(
            projection
                .tree
                .get_by_ref(named(&projection.tree, "Child"))
                .unwrap()
                .properties[&"Transparency".into()],
            Variant::Float32(0.8)
        );
    }

    #[test]
    fn reference_remapping_preserves_accepted_targets_and_rejects_lost_targets() {
        let old = fixture(
            json!({"name":"Root","syncAuthority":[{"className":"ObjectValue","properties":"filesystem"},{"className":"BasePart","properties":"studio","structure":"twoWay"}],"tree":{"$className":"Folder","Target":{"$className":"Part","$id":"target"},"Other":{"$className":"Part"},"Link":{"$className":"ObjectValue","$attributes":{"Rojo_Target_Value":"target"}}}}),
        );
        let mut input = WeakDom::new(InstanceBuilder::new("Folder").with_name("Root"));
        let target = input.insert(
            input.root_ref(),
            InstanceBuilder::new("Part")
                .with_name("Target")
                .with_property("Attributes", Attributes::new().with("Rojo_Id", "target")),
        );
        let other = input.insert(
            input.root_ref(),
            InstanceBuilder::new("Part").with_name("Other"),
        );
        let link = input.insert(
            input.root_ref(),
            InstanceBuilder::new("ObjectValue")
                .with_name("Link")
                .with_property("Value", other),
        );
        let projection = apply(&old, input).unwrap();
        assert_eq!(
            projection.tree.get_by_ref(link).unwrap().properties[&"Value".into()],
            Variant::Ref(target)
        );
        let input = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(InstanceBuilder::new("ObjectValue").with_name("Link")),
        );
        assert!(apply(&old, input)
            .err()
            .unwrap()
            .to_string()
            .contains("Filesystem property"));

        let raw_target = old
            .descendants(old.get_root_id())
            .find(|node| node.name() == "Target")
            .unwrap()
            .id();
        let mut value = Variant::Attributes(
            Attributes::new()
                .with("Target", raw_target)
                .with("Content", Content::from_referent(raw_target)),
        );
        remap(&mut value, &HashMap::from([(raw_target, target)]), 0).unwrap();
        let Variant::Attributes(attributes) = value else {
            unreachable!()
        };
        assert_eq!(attributes.get("Target"), Some(&Variant::Ref(target)));
        assert_eq!(
            attributes.get("Content"),
            Some(&Variant::Content(Content::from_referent(target)))
        );
    }

    #[test]
    fn filesystem_structure_and_ambiguous_matches_refuse_before_serialization() {
        let old = fixture(
            json!({"name":"Root","syncAuthority":[{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","Pole":{"$className":"Part"}}}),
        );
        for input in [
            InstanceBuilder::new("Folder").with_name("Root"),
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(InstanceBuilder::new("MeshPart").with_name("Pole")),
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(InstanceBuilder::new("Part").with_name("Pole"))
                .with_child(InstanceBuilder::new("Part").with_name("Pole")),
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(InstanceBuilder::new("Part").with_name("Pole"))
                .with_child(
                    InstanceBuilder::new("Part")
                        .with_name("Injected")
                        .with_property(
                            "Attributes",
                            Attributes::new().with("syncAuthority", "twoWay"),
                        ),
                ),
        ] {
            assert!(apply(&old, WeakDom::new(input)).is_err());
        }
        assert_eq!(old.root().children().len(), 1);
    }
}
