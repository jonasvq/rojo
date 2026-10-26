use std::collections::{HashMap, HashSet};

use anyhow::{ensure, Context};
use rbx_dom_weak::{
    types::{ContentType, Ref, Variant},
    Instance, InstanceBuilder, Ustr, WeakDom,
};

use crate::{REF_ID_ATTRIBUTE_NAME, REF_POINTER_ATTRIBUTE_PREFIX};

#[derive(Clone, Copy)]
enum Origin {
    Old(Ref, bool),
    New,
}

pub(super) struct Protection {
    attributes: HashMap<Ref, Option<Variant>>,
    context: &'static str,
}

impl Protection {
    pub(super) fn new(attributes: HashMap<Ref, Option<Variant>>, context: &'static str) -> Self {
        Self {
            attributes,
            context,
        }
    }
    pub fn contains(&self, id: Ref) -> bool {
        self.attributes.contains_key(&id)
    }

    pub fn finish_linking(&self, tree: &mut WeakDom) -> anyhow::Result<()> {
        // Normal reference linking generates IDs/markers. Excluded disk data
        // must retain its own attributes, including the absence of attributes.
        for (id, attributes) in &self.attributes {
            let instance = tree
                .get_by_ref_mut(*id)
                .context("An excluded subtree was pruned after projection")?;
            if let Some(attributes) = attributes {
                instance
                    .properties
                    .insert(Ustr::from("Attributes"), attributes.clone());
            } else {
                instance.properties.remove(&Ustr::from("Attributes"));
            }
        }
        for id in super::descendants(tree, tree.root_ref()) {
            let instance = tree.get_by_ref(id).unwrap();
            let attrs = match instance.properties.get(&Ustr::from("Attributes")) {
                Some(Variant::Attributes(attrs)) => Some(attrs),
                _ => None,
            };
            for (property, value) in &instance.properties {
                let target = match value {
                    Variant::Ref(target) => Some(*target),
                    Variant::Content(content) => content.as_object(),
                    _ => None,
                };
                let Some(target) = target.filter(|target| target.is_some()) else {
                    continue;
                };
                let target_instance = tree.get_by_ref(target);
                ensure!(
                    !self.contains(id) || target_instance.is_some(),
                    "{} cannot preserve {}.{} after its target was pruned",
                    self.context,
                    tree.full_path_of(id, "/"),
                    property
                );
                let marker = attrs.and_then(|attrs| {
                    attrs.get(format!("{REF_POINTER_ATTRIBUTE_PREFIX}{property}"))
                });
                if let Some(Variant::String(expected)) = marker {
                    if self.contains(id) || self.contains(target) {
                        ensure!(target_instance.and_then(saved_id) == Some(expected.as_str()),
                            "{} cannot preserve reference identity at {}.{}; retain stable Rojo_Id attributes or change the exclusion", self.context, tree.full_path_of(id, "/"), property);
                    }
                }
            }
        }
        Ok(())
    }
}

fn saved_id(instance: &Instance) -> Option<&str> {
    let Variant::Attributes(attrs) = instance.properties.get(&Ustr::from("Attributes"))? else {
        return None;
    };
    match attrs.get(REF_ID_ATTRIBUTE_NAME)? {
        Variant::String(value) => Some(value),
        Variant::BinaryString(value) => std::str::from_utf8(value.as_ref()).ok(),
        _ => None,
    }
}

fn flags(dom: &WeakDom, classes: &HashSet<Ustr>) -> HashMap<Ref, bool> {
    let mut result = HashMap::new();
    for id in super::descendants(dom, dom.root_ref()).into_iter().rev() {
        let instance = dom.get_by_ref(id).unwrap();
        result.insert(
            id,
            classes.contains(&instance.class)
                || instance.children().iter().any(|child| result[child]),
        );
    }
    result
}

pub(super) fn pairs(
    old: &WeakDom,
    new: &WeakDom,
    old_id: Option<Ref>,
    new_id: Option<Ref>,
    old_flags: &HashMap<Ref, bool>,
    new_flags: &HashMap<Ref, bool>,
    (context, forced): (&str, bool),
) -> anyhow::Result<Vec<(Option<Ref>, Option<Ref>)>> {
    let old_children = old_id
        .map(|id| old.get_by_ref(id).unwrap().children())
        .unwrap_or_default();
    let new_children = new_id
        .map(|id| new.get_by_ref(id).unwrap().children())
        .unwrap_or_default();
    let mut names: HashMap<&str, Vec<Ref>> = HashMap::new();
    let mut ids: HashMap<&str, Vec<Ref>> = HashMap::new();
    let mut new_names: HashMap<&str, usize> = HashMap::new();
    let mut new_ids: HashMap<&str, usize> = HashMap::new();
    for id in old_children {
        let instance = old.get_by_ref(*id).unwrap();
        names.entry(&instance.name).or_default().push(*id);
        if let Some(key) = saved_id(instance) {
            ids.entry(key).or_default().push(*id);
        }
    }
    for id in new_children {
        let instance = new.get_by_ref(*id).unwrap();
        *new_names.entry(&instance.name).or_default() += 1;
        if let Some(key) = saved_id(instance) {
            *new_ids.entry(key).or_default() += 1;
        }
    }
    let mut used = HashSet::new();
    let mut result = Vec::new();
    for id in new_children {
        let instance = new.get_by_ref(*id).unwrap();
        let named = names.get(instance.name.as_str());
        let identified = saved_id(instance).and_then(|key| ids.get(key).map(|list| (key, list)));
        let relevant = forced
            || new_flags[id]
            || named.is_some_and(|list| list.iter().any(|old| old_flags[old]))
            || identified.is_some_and(|(_, list)| list.iter().any(|old| old_flags[old]));
        let matched = if let Some((key, list)) = identified {
            ensure!(
                !relevant || list.len() == 1 && new_ids[key] == 1,
                "{context} cannot preserve duplicated Rojo_Id near {}",
                new.full_path_of(*id, "/")
            );
            (list.len() == 1 && new_ids[key] == 1).then_some(list[0])
        } else {
            None
        };
        let matched = if matched.is_some() {
            matched
        } else if let Some(list) = named {
            ensure!(
                !relevant || list.len() == 1 && new_names[instance.name.as_str()] == 1,
                "{context} cannot preserve ambiguous siblings at {}",
                new.full_path_of(*id, "/")
            );
            (list.len() == 1 && new_names[instance.name.as_str()] == 1).then_some(list[0])
        } else {
            None
        };
        if let Some(old_id) = matched {
            ensure!(
                used.insert(old_id),
                "{context} candidate was matched twice at {}",
                new.full_path_of(*id, "/")
            );
        }
        result.push((matched, Some(*id)));
    }
    result.extend(
        old_children
            .iter()
            .filter(|id| !used.contains(id))
            .map(|id| (Some(*id), None)),
    );
    Ok(result)
}

/// Construct an input view before any writer can serialize an embedded ignored
/// descendant or delete its containing directory. The original DOM stays intact.
pub(super) fn apply(
    old: &WeakDom,
    new: WeakDom,
    ignored: &[Ustr],
) -> anyhow::Result<(WeakDom, Protection)> {
    let database = rbx_reflection_database::get().unwrap();
    for class in ignored {
        ensure!(
            database.classes.contains_key(class.as_str()),
            "Unknown ignoreClasses class {class}"
        );
    }
    let ignored: HashSet<_> = ignored.iter().copied().collect();
    let mut classes = HashSet::new();
    for name in database.classes.keys() {
        let mut current = Some(*name);
        while let Some(class) = current {
            if ignored.contains(&Ustr::from(class)) {
                classes.insert(Ustr::from(name));
                break;
            }
            current = database.classes[class].superclass;
        }
    }
    let old_flags = flags(old, &classes);
    let new_flags = flags(&new, &classes);
    let mut work = vec![(Some(old.root_ref()), Some(new.root_ref()), None, false)];
    let mut output: Option<WeakDom> = None;
    let mut old_map = HashMap::new();
    let mut new_map = HashMap::new();
    let mut origins = HashMap::new();
    while let Some((old_id, new_id, parent, forced)) = work.pop() {
        let old_inst = old_id.and_then(|id| old.get_by_ref(id));
        let new_inst = new_id.and_then(|id| new.get_by_ref(id));
        let preserve = forced
            || old_inst.is_some_and(|inst| classes.contains(&inst.class))
            || new_inst.is_some_and(|inst| classes.contains(&inst.class));
        let (source, origin) = if preserve {
            let Some(instance) = old_inst else { continue };
            (instance, Origin::Old(instance.referent(), true))
        } else if let Some(instance) = new_inst {
            (instance, Origin::New)
        } else {
            let instance = old_inst.unwrap();
            if !old_flags[&instance.referent()] {
                continue;
            }
            (instance, Origin::Old(instance.referent(), false))
        };
        let output_id = old_id.unwrap_or_else(|| new_id.unwrap());
        ensure!(
            !origins.contains_key(&output_id),
            "ignoreClasses encountered colliding instance referents"
        );
        let builder = InstanceBuilder::new(source.class)
            .with_name(&source.name)
            .with_referent(output_id)
            .with_properties(source.properties.clone());
        let id = if let Some(dom) = &mut output {
            dom.insert(
                parent.context("Missing ignoreClasses output parent")?,
                builder,
            )
        } else {
            let dom = WeakDom::new(builder);
            let id = dom.root_ref();
            output = Some(dom);
            id
        };
        if let Some(old_id) = old_id {
            old_map.insert(old_id, id);
        }
        if let Some(new_id) = new_id {
            new_map.insert(new_id, id);
        }
        origins.insert(id, origin);
        let mut children = pairs(
            old,
            &new,
            old_id,
            new_id,
            &old_flags,
            &new_flags,
            ("ignoreClasses", preserve),
        )?;
        // Keep source order while using an iterative stack. A preserved old
        // subtree includes every old child but no newly introduced child.
        if preserve {
            let paired: HashMap<_, _> = children
                .into_iter()
                .filter_map(|(old, new)| old.map(|old| (old, new)))
                .collect();
            children = source
                .children()
                .iter()
                .map(|old| (Some(*old), paired[old]))
                .collect();
        }
        work.extend(
            children
                .into_iter()
                .rev()
                .map(|(old, new)| (old, new, Some(id), preserve)),
        );
    }
    let mut output = output.context("ignoreClasses projection lost its root")?;
    let mut omitted = 0;
    let mut protected_attributes = HashMap::new();
    for (id, origin) in origins {
        let instance = output.get_by_ref_mut(id).unwrap();
        if matches!(origin, Origin::Old(_, true)) {
            protected_attributes.insert(
                id,
                instance.properties.get(&Ustr::from("Attributes")).cloned(),
            );
        }
        let mut cleared = Vec::new();
        for (property, value) in &mut instance.properties {
            let target = match value {
                Variant::Ref(target) => Some(target),
                Variant::Content(content) => match content.value_mut() {
                    ContentType::Object(target) => Some(target),
                    _ => None,
                },
                _ => None,
            };
            if let Some(target) = target {
                if target.is_none() {
                    continue;
                }
                let mapped = match origin {
                    Origin::Old(_, _) => old_map.get(target),
                    Origin::New => new_map.get(target),
                };
                if let Some(mapped) = mapped {
                    *target = *mapped;
                } else {
                    if let Origin::Old(old_id, true) = origin {
                        anyhow::bail!("ignoreClasses cannot preserve reference {}.{} to a removed target; retain the target or change the exclusion", old.full_path_of(old_id, "/"), property);
                    }
                    omitted += 1;
                    *target = Ref::none();
                    cleared.push(format!("{REF_POINTER_ATTRIBUTE_PREFIX}{property}"));
                }
            }
        }
        if let Some(Variant::Attributes(attrs)) =
            instance.properties.get_mut(&Ustr::from("Attributes"))
        {
            for property in cleared {
                attrs.remove(property);
            }
        }
    }
    if omitted > 0 {
        log::warn!(
            "ignoreClasses omitted {omitted} reference targets; their input properties become nil"
        );
    }
    Ok((
        output,
        Protection {
            attributes: protected_attributes,
            context: "ignoreClasses",
        },
    ))
}

#[cfg(test)]
fn project(old: &WeakDom, new: WeakDom, ignored: &[Ustr]) -> anyhow::Result<WeakDom> {
    apply(old, new, ignored).map(|(tree, _)| tree)
}

#[cfg(test)]
mod tests {
    use super::project as project_classes;
    use std::path::Path;

    use memofs::{InMemoryFs, Vfs, VfsSnapshot};
    use rbx_dom_weak::{
        types::{Attributes, Content, Ref, Variant},
        ustr, InstanceBuilder, WeakDom,
    };

    use crate::{
        snapshot::{InstanceContext, RojoTree},
        snapshot_middleware::snapshot_from_vfs,
        syncback::syncback_loop,
        Project,
    };

    fn child(dom: &WeakDom, parent: Ref, name: &str) -> Ref {
        *dom.get_by_ref(parent)
            .unwrap()
            .children()
            .iter()
            .find(|id| dom.get_by_ref(**id).unwrap().name == name)
            .unwrap()
    }

    #[test]
    fn removed_ancestors_keep_only_shells_and_protected_subtrees() {
        let old = WeakDom::new(
            InstanceBuilder::new("Folder").with_child(
                InstanceBuilder::new("Folder")
                    .with_name("Ancestor")
                    .with_child(
                        InstanceBuilder::new("StringValue")
                            .with_name("Protected")
                            .with_property("Value", "disk")
                            .with_child(InstanceBuilder::new("Part").with_name("Inside")),
                    )
                    .with_child(InstanceBuilder::new("Part").with_name("Remove")),
            ),
        );
        let old_count = old
            .get_by_ref(child(&old, old.root_ref(), "Ancestor"))
            .unwrap()
            .children()
            .len();
        let result = project_classes(
            &old,
            WeakDom::new(InstanceBuilder::new("Folder")),
            &[ustr("ValueBase")],
        )
        .unwrap();
        let ancestor = child(&result, result.root_ref(), "Ancestor");
        let protected = child(&result, ancestor, "Protected");
        assert_eq!(result.get_by_ref(ancestor).unwrap().children().len(), 1);
        child(&result, protected, "Inside");
        assert_eq!(old_count, 2, "the original tree was modified");
        assert_eq!(
            old.get_by_ref(child(&old, old.root_ref(), "Ancestor"))
                .unwrap()
                .children()
                .len(),
            2
        );
    }

    #[test]
    fn either_class_exclusion_preserves_old_values_on_class_changes() {
        for (old_class, new_class) in [("StringValue", "Folder"), ("Folder", "StringValue")] {
            let old = WeakDom::new(
                InstanceBuilder::new("Folder").with_child(
                    InstanceBuilder::new(old_class)
                        .with_name("Changed")
                        .with_property("Value", "disk")
                        .with_child(InstanceBuilder::new("Part").with_name("OldChild")),
                ),
            );
            let new = WeakDom::new(
                InstanceBuilder::new("Folder").with_child(
                    InstanceBuilder::new(new_class)
                        .with_name("Changed")
                        .with_property("Value", "input"),
                ),
            );
            let result = project_classes(&old, new, &[ustr("StringValue")]).unwrap();
            let changed = result
                .get_by_ref(child(&result, result.root_ref(), "Changed"))
                .unwrap();
            assert_eq!(changed.class.as_str(), old_class);
            assert_eq!(
                changed.properties[&ustr("Value")],
                Variant::String("disk".into())
            );
            assert_eq!(changed.children().len(), 1);
        }
    }

    #[test]
    fn new_mixed_containers_omit_ignored_descendants_before_serialization() {
        let old = WeakDom::new(InstanceBuilder::new("Folder"));
        let new = WeakDom::new(
            InstanceBuilder::new("Folder").with_child(
                InstanceBuilder::new("Model")
                    .with_name("Mixed")
                    .with_child(InstanceBuilder::new("StringValue").with_name("Omitted"))
                    .with_child(InstanceBuilder::new("Part").with_name("Allowed")),
            ),
        );
        let result = project_classes(&old, new, &[ustr("ValueBase")]).unwrap();
        let mixed = child(&result, result.root_ref(), "Mixed");
        assert_eq!(result.get_by_ref(mixed).unwrap().children().len(), 1);
        child(&result, mixed, "Allowed");
    }

    fn with_id(class: &str, name: &str, id: &str) -> InstanceBuilder {
        let mut attrs = Attributes::new();
        attrs.insert("Rojo_Id".into(), id.into());
        InstanceBuilder::new(class)
            .with_name(name)
            .with_property("Attributes", attrs)
    }

    #[test]
    fn ambiguous_names_or_ids_are_rejected_before_projection_can_write() {
        for by_id in [false, true] {
            let (first, second, incoming) = if by_id {
                (
                    with_id("StringValue", "First", "DUP"),
                    with_id("StringValue", "Second", "DUP"),
                    with_id("StringValue", "Other", "DUP"),
                )
            } else {
                (
                    InstanceBuilder::new("StringValue").with_name("Same"),
                    InstanceBuilder::new("StringValue").with_name("Same"),
                    InstanceBuilder::new("StringValue").with_name("Same"),
                )
            };
            let old = WeakDom::new(
                InstanceBuilder::new("Folder")
                    .with_child(first)
                    .with_child(second),
            );
            let new = WeakDom::new(InstanceBuilder::new("Folder").with_child(incoming));
            let err = project_classes(&old, new, &[ustr("StringValue")]).unwrap_err();
            assert!(
                err.to_string()
                    .contains(if by_id { "Rojo_Id" } else { "ambiguous" }),
                "{err}"
            );
        }
    }

    #[test]
    fn unique_saved_ids_disambiguate_equal_names_and_preserve_renames() {
        let old = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_child(with_id("StringValue", "Same", "A").with_property("Value", "a"))
                .with_child(with_id("StringValue", "Same", "B").with_property("Value", "b")),
        );
        let new = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_child(with_id("StringValue", "Renamed", "B"))
                .with_child(with_id("StringValue", "Same", "A")),
        );
        let result = project_classes(&old, new, &[ustr("StringValue")]).unwrap();
        let values: Vec<_> = result
            .root()
            .children()
            .iter()
            .map(|id| {
                let instance = result.get_by_ref(*id).unwrap();
                assert_eq!(instance.name, "Same");
                instance.properties[&ustr("Value")].clone()
            })
            .collect();
        assert_eq!(
            values,
            [Variant::String("b".into()), Variant::String("a".into())]
        );
    }

    #[test]
    fn references_cycles_content_and_omitted_target_markers_are_remapped() {
        let (a, d, b) = (Ref::new(), Ref::new(), Ref::new());
        let old = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_child(
                    InstanceBuilder::new("ObjectValue")
                        .with_name("A")
                        .with_referent(a)
                        .with_property("Value", d)
                        .with_property("External", b),
                )
                .with_child(
                    InstanceBuilder::new("ObjectValue")
                        .with_name("D")
                        .with_referent(d)
                        .with_property("Value", a),
                )
                .with_child(InstanceBuilder::new("Part").with_name("B").with_referent(b)),
        );
        let (na, nd, nb, omitted) = (Ref::new(), Ref::new(), Ref::new(), Ref::new());
        let mut attrs = Attributes::new();
        attrs.insert("Rojo_Target_Omitted".into(), "stale target".into());
        let new = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_child(
                    InstanceBuilder::new("ObjectValue")
                        .with_name("A")
                        .with_referent(na),
                )
                .with_child(
                    InstanceBuilder::new("ObjectValue")
                        .with_name("D")
                        .with_referent(nd),
                )
                .with_child(
                    InstanceBuilder::new("ObjectValue")
                        .with_name("Omitted")
                        .with_referent(omitted),
                )
                .with_child(
                    InstanceBuilder::new("Part")
                        .with_name("B")
                        .with_referent(nb)
                        .with_property("Pointer", na)
                        .with_property("ContentPointer", Content::from_referent(na))
                        .with_property("Omitted", omitted)
                        .with_property("Nil", Ref::none())
                        .with_property("Attributes", attrs),
                ),
        );
        let result = project_classes(&old, new, &[ustr("ObjectValue")]).unwrap();
        let props = &result.get_by_ref(b).unwrap().properties;
        assert_eq!(props[&ustr("Pointer")], Variant::Ref(a));
        assert_eq!(
            props[&ustr("ContentPointer")],
            Variant::Content(Content::from_referent(a))
        );
        assert_eq!(props[&ustr("Omitted")], Variant::Ref(Ref::none()));
        assert_eq!(props[&ustr("Nil")], Variant::Ref(Ref::none()));
        let Variant::Attributes(attrs) = &props[&ustr("Attributes")] else {
            panic!("missing attributes")
        };
        assert!(attrs.get("Rojo_Target_Omitted").is_none());
        assert_eq!(
            result.get_by_ref(a).unwrap().properties[&ustr("Value")],
            Variant::Ref(d)
        );
        assert_eq!(
            result.get_by_ref(a).unwrap().properties[&ustr("External")],
            Variant::Ref(b)
        );
        assert_eq!(
            result.get_by_ref(d).unwrap().properties[&ustr("Value")],
            Variant::Ref(a)
        );
    }

    #[test]
    fn removed_targets_needed_by_preserved_data_and_unknown_classes_fail() {
        let target = Ref::new();
        let old = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_child(
                    InstanceBuilder::new("ObjectValue")
                        .with_name("Protected")
                        .with_property("Value", target),
                )
                .with_child(
                    InstanceBuilder::new("Part")
                        .with_name("Removed")
                        .with_referent(target),
                ),
        );
        let new = || WeakDom::new(InstanceBuilder::new("Folder"));
        let error = project_classes(&old, new(), &[ustr("ObjectValue")]).unwrap_err();
        assert!(error.to_string().contains("Protected.Value"), "{error}");
        let error = project_classes(&old, new(), &[ustr("MisspelledClass")]).unwrap_err();
        assert!(error.to_string().contains("Unknown ignoreClasses class"));
    }

    #[test]
    fn reference_linking_cannot_invent_identity_in_protected_source_attributes() {
        use crate::syncback::ref_properties::{collect_referents, link_referents};
        for stable_id in [false, true] {
            let target = if stable_id {
                with_id("Part", "Protected", "stable-target")
            } else {
                InstanceBuilder::new("Part").with_name("Protected")
            };
            let old = WeakDom::new(InstanceBuilder::new("Folder").with_child(target));
            let target_ref = Ref::new();
            let input = WeakDom::new(
                InstanceBuilder::new("Folder")
                    .with_child(
                        InstanceBuilder::new("Part")
                            .with_name("Protected")
                            .with_referent(target_ref),
                    )
                    .with_child(
                        InstanceBuilder::new("ObjectValue")
                            .with_name("Pointer")
                            .with_property("Value", target_ref),
                    ),
            );
            let (mut projected, protection) =
                super::apply(&old, input, &[ustr("BasePart")]).unwrap();
            let links = collect_referents(&projected);
            link_referents(links, &mut projected).unwrap();
            let result = protection.finish_linking(&mut projected);
            if stable_id {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("stable Rojo_Id"));
            }
            let protected = projected
                .get_by_ref(child(&projected, projected.root_ref(), "Protected"))
                .unwrap();
            assert_eq!(
                super::saved_id(protected),
                stable_id.then_some("stable-target")
            );
        }
    }

    #[test]
    fn embedded_binary_xml_and_json_subtrees_survive_allowed_updates_and_repeat_runs() {
        for extension in ["rbxm", "rbxmx", "model.json"] {
            for exclude_root in [false, true] {
                let filename = format!("container.{extension}");
                let old_dom = WeakDom::new(
                    InstanceBuilder::new("Folder")
                        .with_child(
                            InstanceBuilder::new("StringValue")
                                .with_name("Protected")
                                .with_property("Value", "disk"),
                        )
                        .with_child(InstanceBuilder::new("Part").with_name("Allowed")),
                );
                let mut bytes = Vec::new();
                match extension {
                    "rbxm" => rbx_binary::to_writer(&mut bytes, &old_dom, &[old_dom.root_ref()]).unwrap(),
                    "rbxmx" => rbx_xml::to_writer(&mut bytes, &old_dom, &[old_dom.root_ref()], rbx_xml::EncodeOptions::new()).unwrap(),
                    _ => bytes.extend_from_slice(br#"{"className":"Folder","children":[{"name":"Protected","className":"StringValue","properties":{"Value":"disk"}},{"name":"Allowed","className":"Part"}]}"#),
                }
                let contents = serde_json::to_vec(&serde_json::json!({
                    "name":"Root", "tree":{"$path":filename},
                    "syncbackRules":{"ignoreClasses":[if exclude_root { "Folder" } else { "ValueBase" }]}
                })).unwrap();
                let mut fs = InMemoryFs::new();
                fs.load_snapshot(
                    "/project",
                    VfsSnapshot::dir([
                        ("default.project.json", VfsSnapshot::file(contents)),
                        (filename.as_str(), VfsSnapshot::file(bytes.clone())),
                    ]),
                )
                .unwrap();
                let vfs = Vfs::new(fs);
                vfs.set_watch_enabled(false);
                let path = Path::new("/project/default.project.json");
                for iteration in 0..2 {
                    let project = Project::load_initial_project(&vfs, path).unwrap();
                    let old_snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                        .unwrap()
                        .unwrap();
                    let mut old = RojoTree::new(old_snapshot);
                    let new = WeakDom::new(
                        InstanceBuilder::new("Folder")
                            .with_name("Root")
                            .with_child(
                                InstanceBuilder::new("StringValue")
                                    .with_name("Protected")
                                    .with_property("Value", "input"),
                            )
                            .with_child(
                                InstanceBuilder::new("Part")
                                    .with_name("Allowed")
                                    .with_property("Anchored", true),
                            )
                            .with_child(InstanceBuilder::new("StringValue").with_name("Omitted")),
                    );
                    let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
                    if iteration == 1 || exclude_root {
                        assert!(
                            changes.is_empty(),
                            "{extension}, root={exclude_root}, iteration={iteration}"
                        );
                    }
                    assert!(changes.removed_paths().is_empty());
                    changes.write_to_vfs("/project", &vfs).unwrap();
                    let after = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                        .unwrap()
                        .unwrap();
                    assert_eq!(after.children.len(), 2);
                    let protected = after
                        .children
                        .iter()
                        .find(|child| child.name == "Protected")
                        .unwrap();
                    assert_eq!(
                        protected.properties[&ustr("Value")],
                        Variant::String("disk".into())
                    );
                    let allowed = after
                        .children
                        .iter()
                        .find(|child| child.name == "Allowed")
                        .unwrap();
                    assert_eq!(
                        allowed
                            .properties
                            .get(&ustr("Anchored"))
                            .cloned()
                            .unwrap_or(Variant::Bool(false)),
                        Variant::Bool(!exclude_root)
                    );
                    if exclude_root {
                        assert_eq!(
                            vfs.read(Path::new("/project").join(&filename))
                                .unwrap()
                                .as_slice(),
                            bytes.as_slice()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn inline_project_exclusions_keep_disk_values_while_other_properties_change() {
        let contents = br#"{"name":"Root","syncbackRules":{"ignoreClasses":["StringValue"]},"tree":{"$className":"Folder","Protected":{"$className":"StringValue","$properties":{"Value":"disk"}},"Allowed":{"$className":"Part"}}}"#;
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/project/default.project.json",
            VfsSnapshot::file(contents.to_vec()),
        )
        .unwrap();
        let vfs = Vfs::new(fs);
        let path = Path::new("/project/default.project.json");
        let project = Project::load_initial_project(&vfs, path).unwrap();
        let old_snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(old_snapshot);
        let new = WeakDom::new(
            InstanceBuilder::new("Folder").with_name("Root").with_child(
                InstanceBuilder::new("Part")
                    .with_name("Allowed")
                    .with_property("Anchored", true),
            ),
        );
        let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
        changes.write_to_vfs("/project", &vfs).unwrap();
        let after = Project::load_initial_project(&vfs, path).unwrap();
        assert_eq!(
            serde_json::to_value(&after.tree.children["Protected"].properties[&ustr("Value")])
                .unwrap(),
            "disk"
        );
        assert_eq!(
            serde_json::to_value(&after.tree.children["Allowed"].properties[&ustr("Anchored")])
                .unwrap(),
            true
        );
        assert_eq!(
            serde_json::to_value(after.syncback_rules).unwrap()["ignoreClasses"],
            serde_json::json!(["StringValue"])
        );
    }

    #[test]
    fn directory_deletions_preserve_excluded_descendants_and_remove_allowed_siblings() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(br#"{"name":"Root","tree":{"$path":"src"},"syncbackRules":{"ignoreClasses":["StringValue"]}}"#.to_vec())),
            ("src", VfsSnapshot::dir([("Ancestor", VfsSnapshot::dir([
                ("Protected.txt", VfsSnapshot::file("disk")),
                ("Remove.luau", VfsSnapshot::file("return 1")),
            ]))])),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        let path = Path::new("/project/default.project.json");
        let project = Project::load_initial_project(&vfs, path).unwrap();
        let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(snapshot);
        let new = WeakDom::new(InstanceBuilder::new("Folder").with_name("Root"));
        let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
        assert_eq!(
            changes.removed_paths(),
            [Path::new("/project/src/Ancestor/Remove.luau")]
        );
        changes.write_to_vfs("/project", &vfs).unwrap();
        assert_eq!(
            vfs.read("/project/src/Ancestor/Protected.txt")
                .unwrap()
                .as_slice(),
            b"disk"
        );
        assert!(vfs.metadata("/project/src/Ancestor/Remove.luau").is_err());
    }

    #[test]
    fn native_script_directories_keep_sources_and_all_children_across_class_changes() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(br#"{"name":"Root","tree":{"$path":"src"},"syncbackRules":{"ignoreClasses":["LuaSourceContainer"]}}"#.to_vec())),
            ("src", VfsSnapshot::dir([("Container", VfsSnapshot::dir([
                ("init.luau", VfsSnapshot::file("return 'disk'\r\n")),
                ("Inside.txt", VfsSnapshot::file("child disk")),
            ]))])),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        let path = Path::new("/project/default.project.json");
        let project = Project::load_initial_project(&vfs, path).unwrap();
        let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(snapshot);
        let new = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(
                    InstanceBuilder::new("Script")
                        .with_name("Container")
                        .with_property("Source", "input"),
                )
                .with_child(
                    InstanceBuilder::new("ModuleScript")
                        .with_name("Omitted")
                        .with_property("Source", "input"),
                ),
        );
        let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
        assert!(
            changes.is_empty(),
            "native excluded directory was rewritten"
        );
        assert_eq!(
            vfs.read("/project/src/Container/init.luau")
                .unwrap()
                .as_slice(),
            b"return 'disk'\r\n"
        );
        assert_eq!(
            vfs.read("/project/src/Container/Inside.txt")
                .unwrap()
                .as_slice(),
            b"child disk"
        );
    }

    #[test]
    fn excluded_current_camera_survives_default_camera_pruning() {
        let camera = Ref::new();
        let workspace = WeakDom::new(
            InstanceBuilder::new("Workspace")
                .with_property("CurrentCamera", camera)
                .with_child(
                    with_id("Camera", "Cam", "camera-stable")
                        .with_referent(camera)
                        .with_property("FieldOfView", 75.0f32),
                )
                .with_child(InstanceBuilder::new("Part").with_name("Allowed")),
        );
        let mut bytes = Vec::new();
        rbx_binary::to_writer(&mut bytes, &workspace, &[workspace.root_ref()]).unwrap();
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(br#"{"name":"Root","tree":{"$className":"DataModel","Workspace":{"$path":"workspace.rbxm"}},"syncbackRules":{"ignoreClasses":["Camera"]}}"#.to_vec())),
            ("workspace.rbxm", VfsSnapshot::file(bytes)),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        let path = Path::new("/project/default.project.json");
        let project = Project::load_initial_project(&vfs, path).unwrap();
        let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(snapshot);
        let input_camera = Ref::new();
        let input = WeakDom::new(
            InstanceBuilder::new("DataModel")
                .with_name("Root")
                .with_child(
                    InstanceBuilder::new("Workspace")
                        .with_property("CurrentCamera", input_camera)
                        .with_child(
                            InstanceBuilder::new("Camera")
                                .with_name("Cam")
                                .with_referent(input_camera)
                                .with_property("FieldOfView", 30.0f32),
                        )
                        .with_child(
                            InstanceBuilder::new("Part")
                                .with_name("Allowed")
                                .with_property("Anchored", true),
                        ),
                ),
        );
        let changes = syncback_loop(&vfs, &mut old, input, &project).unwrap();
        changes.write_to_vfs("/project", &vfs).unwrap();
        let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let workspace = &snapshot.children[0];
        let camera = workspace
            .children
            .iter()
            .find(|child| child.name == "Cam")
            .expect("excluded camera was pruned");
        assert_eq!(
            camera.properties[&ustr("FieldOfView")],
            Variant::Float32(75.0)
        );
        assert_eq!(workspace.children.len(), 2);
    }

    #[test]
    fn empty_rules_remain_omitted_and_invalid_class_names_fail_without_writes() {
        let rules: crate::syncback::SyncbackRules = serde_json::from_str("{}").unwrap();
        assert!(serde_json::to_value(rules)
            .unwrap()
            .get("ignoreClasses")
            .is_none());
        let contents = br#"{"name":"Root","tree":{"$path":"src"},"syncbackRules":{"ignoreClasses":["NoSuchClass"]}}"#;
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/project",
            VfsSnapshot::dir([
                ("default.project.json", VfsSnapshot::file(contents.to_vec())),
                (
                    "src",
                    VfsSnapshot::dir([("Keep.txt", VfsSnapshot::file("unchanged"))]),
                ),
            ]),
        )
        .unwrap();
        let vfs = Vfs::new(fs);
        let path = Path::new("/project/default.project.json");
        let project = Project::load_initial_project(&vfs, path).unwrap();
        let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(snapshot);
        let error = syncback_loop(
            &vfs,
            &mut old,
            WeakDom::new(InstanceBuilder::new("Folder")),
            &project,
        )
        .err()
        .unwrap();
        let message = format!("{error:#}");
        assert!(
            message.contains("NoSuchClass") && message.contains("/project/default.project.json"),
            "{message}"
        );
        assert_eq!(
            vfs.read("/project/src/Keep.txt").unwrap().as_slice(),
            b"unchanged"
        );
        assert_eq!(vfs.read(path).unwrap().as_slice(), contents);
    }

    #[test]
    fn exclusions_preserve_properties_and_do_not_bypass_other_path_rules() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(br#"{"name":"Root","tree":{"$path":"src"},"syncbackRules":{"ignoreClasses":["StringValue"],"ignoreProperties":{"StringValue":["Value"]},"ignorePaths":["src/Keep.luau"]}}"#.to_vec())),
            ("src", VfsSnapshot::dir([
                ("Protected.txt", VfsSnapshot::file("disk")),
                ("Keep.luau", VfsSnapshot::file("return 1")),
            ])),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        let path = Path::new("/project/default.project.json");
        let project = Project::load_initial_project(&vfs, path).unwrap();
        let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(snapshot);
        let new = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(
                    InstanceBuilder::new("StringValue")
                        .with_name("Protected")
                        .with_property("Value", "input"),
                )
                .with_child(
                    InstanceBuilder::new("ModuleScript")
                        .with_name("Keep")
                        .with_property("Source", "return 2"),
                ),
        );
        let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
        assert!(
            changes.is_empty(),
            "class exclusions bypassed an existing no-write rule"
        );
        assert_eq!(
            vfs.read("/project/src/Protected.txt").unwrap().as_slice(),
            b"disk"
        );
        assert_eq!(
            vfs.read("/project/src/Keep.luau").unwrap().as_slice(),
            b"return 1"
        );
    }

    #[test]
    fn ignored_classes_preserve_old_sources_and_omit_new_subtrees() {
        for incoming_old in [false, true] {
            let mut fs = InMemoryFs::new();
            fs.load_snapshot("/project", VfsSnapshot::dir([
                ("default.project.json", VfsSnapshot::file(br#"{"name":"Root","tree":{"$path":"src"},"syncbackRules":{"ignoreClasses":["ValueBase"]}}"#.to_vec())),
                ("src", VfsSnapshot::dir([
                    ("Old.txt", VfsSnapshot::file("disk value")),
                    ("Keep.luau", VfsSnapshot::file("return 1")),
                ])),
            ])).unwrap();
            let vfs = Vfs::new(fs);
            vfs.set_watch_enabled(false);
            let path = Path::new("/project/default.project.json");
            let project =
                Project::load_initial_project(&vfs, path).expect("ignoreClasses must parse");
            let old_snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                .unwrap()
                .unwrap();
            let mut old = RojoTree::new(old_snapshot);
            let mut root = InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(
                    InstanceBuilder::new("ModuleScript")
                        .with_name("Keep")
                        .with_property("Source", "return 2"),
                )
                .with_child(
                    InstanceBuilder::new("StringValue")
                        .with_name("New")
                        .with_property("Value", "omit")
                        .with_child(InstanceBuilder::new("Folder").with_name("AlsoOmitted")),
                );
            if incoming_old {
                root = root.with_child(
                    InstanceBuilder::new("StringValue")
                        .with_name("Old")
                        .with_property("Value", "input value"),
                );
            }
            let changes = syncback_loop(&vfs, &mut old, WeakDom::new(root), &project).unwrap();
            assert!(changes.removed_paths().is_empty());
            changes.write_to_vfs("/project", &vfs).unwrap();
            assert_eq!(
                vfs.read("/project/src/Old.txt").unwrap().as_slice(),
                b"disk value"
            );
            assert_eq!(
                vfs.read("/project/src/Keep.luau").unwrap().as_slice(),
                b"return 2"
            );
            let after = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                .unwrap()
                .unwrap();
            assert_eq!(after.children.len(), 2);
            assert!(after.children.iter().all(|child| child.name != "New"));
        }
    }
}
