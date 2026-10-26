use librojo::{snapshot_from_vfs, InstanceContext, InstanceSnapshot};
use memofs::{InMemoryFs, Vfs, VfsBackend, VfsSnapshot};
use rbx_dom_weak::{types::Variant, ustr};
use serde_json::json;

fn snapshot(project: &str, files: &[(&str, &str)]) -> InstanceSnapshot {
    let mut fs = InMemoryFs::new();
    fs.create_dir_all(std::path::Path::new("/project")).unwrap();
    fs.load_snapshot("/project/default.project.json", VfsSnapshot::file(project))
        .unwrap();
    for (name, contents) in files {
        fs.create_dir_all(
            std::path::Path::new(&format!("/project/{name}"))
                .parent()
                .unwrap(),
        )
        .unwrap();
        fs.load_snapshot(format!("/project/{name}"), VfsSnapshot::file(*contents))
            .unwrap();
    }
    let vfs = Vfs::new(fs);
    vfs.set_watch_enabled(false);
    snapshot_from_vfs(
        &InstanceContext::default(),
        &vfs,
        std::path::Path::new("/project/default.project.json"),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn csv_inherits_preservation_default_but_explicit_metadata_and_rules_win() {
    for (meta, rules, expected) in [
        ("{}", "[]", true),
        (r#"{"ignoreUnknownInstances":false}"#, "[]", false),
        (r#"{"ignoreUnknownInstances":true}"#, "[]", true),
        (
            "{}",
            r#"[{"pattern":"*.csv","defaults":{"ignoreUnknownInstances":false}}]"#,
            false,
        ),
        (
            r#"{"ignoreUnknownInstances":false}"#,
            r#"[{"pattern":"*.csv","defaults":{"ignoreUnknownInstances":true}}]"#,
            false,
        ),
    ] {
        let root = snapshot(
            &format!(
                r#"{{"$version":2,"name":"Root","syncRules":{rules},"tree":{{"$className":"Folder","Table":{{"$path":"Table.csv"}}}}}}"#
            ),
            &[
                ("Table.csv", "Key,Source\nhello,world\n"),
                ("Table.meta.jsonc", meta),
            ],
        );
        assert_eq!(
            root.children[0].metadata.ignore_unknown_instances, expected,
            "meta={meta}, rules={rules}"
        );
    }
    let root = snapshot(
        r#"{"$version":2,"name":"Root","tree":{"$className":"Folder","Table":{"$path":"Table.csv"}}}"#,
        &[
            ("Table.csv", "Key,Source\nhello,world\n"),
            ("Table.meta.json", r#"{"ignoreUnknownInstances":false}"#),
            ("Table.meta.jsonc", r#"{"ignoreUnknownInstances":null}"#),
        ],
    );
    assert!(!root.children[0].metadata.ignore_unknown_instances);
}

#[test]
fn defaults_only_rules_tag_modules_without_intercepting_builtin_routing() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"*.luau","defaults":{"tags":["SpecialModule"],"attributes":{"Owner":"shared"}}}],"tree":{"$className":"Folder","Module":{"$path":"Module.luau"}}}"#,
        &[("Module.luau", "return 1")],
    );
    let module = &root.children[0];
    assert_eq!(module.class_name, "ModuleScript");
    assert_eq!(
        module.properties[&ustr("Source")],
        Variant::String("return 1".into())
    );
    let Variant::Tags(tags) = &module.properties[&ustr("Tags")] else {
        panic!("missing tags")
    };
    assert_eq!(tags.iter().collect::<Vec<_>>(), ["SpecialModule"]);
    let Variant::Attributes(attrs) = &module.properties[&ustr("Attributes")] else {
        panic!("missing attributes")
    };
    assert_eq!(attrs.get("Owner"), Some(&Variant::String("shared".into())));
}

#[test]
fn csv_defaults_use_the_common_boundary_and_preserve_jsonc_overrides() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{
        types::{Attributes, Tags},
        InstanceBuilder, WeakDom,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/Table.csv"),
        "Key,Source\nhello,world\n",
    )
    .unwrap();
    let meta_path = dir.path().join("src/Table.meta.jsonc");
    std::fs::write(&meta_path, r#"{"attributes":{"Owner":"explicit"}}"#).unwrap();
    let path = dir.path().join("default.project.json");
    std::fs::write(&path, r#"{"name":"Root","syncRules":[{"pattern":"*.csv","defaults":{"tags":["Shared"],"attributes":{"Owner":"shared","Other":true}}}],"tree":{"$path":"src"}}"#).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = build_project(&vfs, &project).unwrap();
    let original = old
        .inner()
        .get_by_ref(old.inner().root().children()[0])
        .unwrap();
    assert!(original.properties.contains_key(&ustr("Tags")));
    let mut attrs = Attributes::new();
    attrs.insert("Owner".into(), Variant::String("explicit".into()));
    attrs.insert("Other".into(), Variant::Bool(true));
    let mut tags = Tags::new();
    tags.push("Shared");
    let new = WeakDom::new(
        InstanceBuilder::new("Folder").with_name("Root").with_child(
            InstanceBuilder::new("LocalizationTable")
                .with_name("Table")
                .with_property(
                    "Contents",
                    r#"[{"key":"hello","source":"new","values":{}}]"#,
                )
                .with_property("Attributes", attrs)
                .with_property("Tags", tags),
        ),
    );
    let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
    changes.write_to_vfs(dir.path(), &vfs).unwrap();
    assert!(!dir.path().join("src/Table.meta.json").exists());
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(meta_path).unwrap()).unwrap();
    assert_eq!(meta, json!({"attributes":{"Owner":"explicit"}}));
    assert!(std::fs::read_to_string(dir.path().join("src/Table.csv"))
        .unwrap()
        .contains("new"));
}

#[test]
fn explicit_empty_false_and_attribute_values_override_only_their_own_defaults() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"*.luau","use":"moduleScript","defaults":{"tags":["SpecialModule"],"properties":{"Archivable":true},"attributes":{"Owner":"shared","Count":5},"ignoreUnknownInstances":true}}],"tree":{"$className":"Folder","Module":{"$path":"Module.luau"}}}"#,
        &[
            ("Module.luau", "return 1"),
            (
                "Module.meta.jsonc",
                r#"{"properties":{"Tags":[],"Archivable":false},"attributes":{"Count":0},"ignoreUnknownInstances":false}"#,
            ),
        ],
    );
    let module = &root.children[0];
    let Variant::Tags(tags) = &module.properties[&ustr("Tags")] else {
        panic!("missing tags")
    };
    assert_eq!(tags.iter().count(), 0);
    assert_eq!(module.properties[&ustr("Archivable")], Variant::Bool(false));
    assert!(!module.metadata.ignore_unknown_instances);
    let Variant::Attributes(attrs) = &module.properties[&ustr("Attributes")] else {
        panic!("missing attributes")
    };
    assert_eq!(attrs.get("Owner"), Some(&Variant::String("shared".into())));
    assert_eq!(attrs.get("Count"), Some(&Variant::Float64(0.0)));
}

#[test]
fn overlapping_defaults_use_first_matching_key_while_mapping_rules_still_route() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"*","defaults":{"tags":["First"],"attributes":{"Owner":"first"}}},{"pattern":"*.custom","use":"moduleScript","suffix":".custom","defaults":{"tags":["Second"],"attributes":{"Owner":"second","Extra":true}}}],"tree":{"$className":"Folder","Module":{"$path":"Module.custom"}}}"#,
        &[("Module.custom", "return 2")],
    );
    let module = &root.children[0];
    assert_eq!(module.class_name, "ModuleScript");
    let Variant::Tags(tags) = &module.properties[&ustr("Tags")] else {
        panic!("missing tags")
    };
    assert_eq!(tags.iter().collect::<Vec<_>>(), ["First"]);
    let Variant::Attributes(attrs) = &module.properties[&ustr("Attributes")] else {
        panic!("missing attributes")
    };
    assert_eq!(attrs.get("Owner"), Some(&Variant::String("first".into())));
    assert_eq!(attrs.get("Extra"), Some(&Variant::Bool(true)));
}

#[test]
fn masks_and_project_overrides_preserve_other_shared_keys() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"*.luau","defaults":{"tags":["Shared"],"attributes":{"Owner":"shared","Other":true},"ignoreUnknownInstances":true}}],"tree":{"$className":"Folder","Module":{"$path":"Module.luau","$ruleDefaults":{"disabledProperties":["Tags"],"disabledAttributes":["Owner"],"disableIgnoreUnknownInstances":true},"$attributes":{"Other":false}}}}"#,
        &[("Module.luau", "return 1")],
    );
    let module = &root.children[0];
    assert!(!module.properties.contains_key(&ustr("Tags")));
    assert!(!module.metadata.ignore_unknown_instances);
    let Variant::Attributes(attrs) = &module.properties[&ustr("Attributes")] else {
        panic!()
    };
    assert_eq!(attrs.get("Owner"), None);
    assert_eq!(attrs.get("Other"), Some(&Variant::Bool(false)));
}

#[test]
fn invalid_rules_are_rejected_before_snapshotting() {
    for rule in [
        json!({"pattern":"*"}),
        json!({"pattern":"*","defaults":{}}),
        json!({"pattern":"*","defaults":{"attributes":{}},"suffix":".lua"}),
        json!({"pattern":"*","defaults":{"typo":true}}),
        json!({"pattern":"*","defaults":{"tags":["A"],"properties":{"Tags":[]}}}),
        json!({"pattern":"*","defaults":{"properties":{"Source":"bad"}}}),
        json!({"pattern":"*","defaults":{"properties":{"Attributes":{}}}}),
    ] {
        assert!(
            serde_json::from_value::<librojo::Project>(
                json!({"tree":{"$className":"Folder"},"syncRules":[rule.clone()]})
            )
            .is_err(),
            "accepted {rule}"
        );
    }
}

#[test]
fn init_rules_match_source_files_and_do_not_leak_to_embedded_children() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"**/init.luau","defaults":{"tags":["Init"]}},{"pattern":"*.model.json","defaults":{"tags":["Model"]}}],"tree":{"$className":"Folder","Scripts":{"$path":"Scripts"},"Model":{"$path":"Model.model.json"}}}"#,
        &[
            ("Scripts/init.luau", "return 1"),
            ("Scripts/Child.luau", "return 2"),
            (
                "Model.model.json",
                r#"{"className":"Folder","children":[{"className":"Folder","name":"Child"}]}"#,
            ),
        ],
    );
    for child in &root.children {
        assert!(
            child.properties.contains_key(&ustr("Tags")),
            "{}",
            child.name
        );
        assert!(!child.children[0].properties.contains_key(&ustr("Tags")));
    }
}

#[test]
fn explicit_model_values_and_attributes_override_fallbacks_per_key() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"*.model.json","defaults":{"tags":["Shared"],"attributes":{"Owner":"shared","Extra":true},"className":"Part"}}],"tree":{"$className":"Folder","Model":{"$path":"Model.model.json"}}}"#,
        &[(
            "Model.model.json",
            r#"{"className":"Folder","properties":{"Tags":[]},"attributes":{"Owner":"explicit"}}"#,
        )],
    );
    let model = &root.children[0];
    assert_eq!(model.class_name, "Folder");
    let Variant::Tags(tags) = &model.properties[&ustr("Tags")] else {
        panic!()
    };
    assert_eq!(tags.iter().count(), 0);
    let Variant::Attributes(attrs) = &model.properties[&ustr("Attributes")] else {
        panic!()
    };
    assert_eq!(
        attrs.get("Owner"),
        Some(&Variant::String("explicit".into()))
    );
    assert_eq!(attrs.get("Extra"), Some(&Variant::Bool(true)));
}

#[test]
fn class_defaults_are_applied_before_class_specific_metadata() {
    let root = snapshot(
        r#"{"name":"Root","emitLegacyScripts":true,"syncRules":[{"pattern":"Scripts","defaults":{"className":"Part"}},{"pattern":"*.client.luau","defaults":{"className":"Script","properties":{"RunContext":"Client"}}}],"tree":{"$className":"Folder","Scripts":{"$path":"Scripts"},"Client":{"$path":"Client.client.luau"}}}"#,
        &[
            (
                "Scripts/init.meta.json",
                r#"{"properties":{"Transparency":0.5}}"#,
            ),
            ("Client.client.luau", "print('hi')"),
            (
                "Client.meta.json",
                r#"{"properties":{"RunContext":"Server"}}"#,
            ),
        ],
    );
    let part = root.children.iter().find(|s| s.name == "Scripts").unwrap();
    assert_eq!(part.class_name, "Part");
    assert_eq!(
        part.properties[&ustr("Transparency")],
        Variant::Float32(0.5)
    );
    let client = root.children.iter().find(|s| s.name == "Client").unwrap();
    assert_eq!(client.class_name, "Script");
    assert_eq!(
        client.properties[&ustr("RunContext")],
        Variant::Enum(rbx_dom_weak::types::Enum::from_u32(1))
    );
}

#[test]
fn explicit_project_class_wins_and_source_context_does_not_retain_owner_hints() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"Scripts","defaults":{"className":"Model","properties":{"Transparency":0.2}}}],"tree":{"$className":"Folder","Scripts":{"$path":"Scripts","$className":"Part"}}}"#,
        &[(
            "Scripts/init.meta.json",
            r#"{"properties":{"Transparency":0.5}}"#,
        )],
    );
    let part = &root.children[0];
    assert_eq!(part.class_name, "Part");
    assert_eq!(
        part.properties[&ustr("Transparency")],
        Variant::Float32(0.5)
    );
    assert!(part.metadata.context.rule_owner.is_none());
}

#[test]
fn class_mask_restores_original_modern_script_context() {
    let root = snapshot(
        r#"{"name":"Root","emitLegacyScripts":false,"syncRules":[{"pattern":"*.client.luau","defaults":{"className":"LocalScript"}}],"tree":{"$className":"Folder","Client":{"$path":"Client.client.luau","$ruleDefaults":{"disableClassName":true}}}}"#,
        &[("Client.client.luau", "print('hi')")],
    );
    let client = &root.children[0];
    assert_eq!(client.class_name, "Script");
    assert_eq!(
        client.properties[&ustr("RunContext")],
        Variant::Enum(rbx_dom_weak::types::Enum::from_u32(2))
    );
}

#[test]
fn excluded_rules_and_nested_project_resets_keep_their_existing_scope() {
    let root = snapshot(
        r#"{"name":"Root","syncRules":[{"pattern":"*.luau","exclude":"Skip.luau","defaults":{"tags":["Outer"]}}],"tree":{"$className":"Folder","Keep":{"$path":"Keep.luau"},"Skip":{"$path":"Skip.luau"},"Nested":{"$path":"Nested"}}}"#,
        &[
            ("Keep.luau", "return 1"),
            ("Skip.luau", "return 2"),
            (
                "Nested/default.project.json",
                r#"{"tree":{"$className":"Folder","Inner":{"$path":"Inner.luau"}}}"#,
            ),
            ("Nested/Inner.luau", "return 3"),
        ],
    );
    let keep = root.children.iter().find(|s| s.name == "Keep").unwrap();
    let skip = root.children.iter().find(|s| s.name == "Skip").unwrap();
    let nested = root.children.iter().find(|s| s.name == "Nested").unwrap();
    assert!(keep.properties.contains_key(&ustr("Tags")));
    assert!(!skip.properties.contains_key(&ustr("Tags")));
    assert!(!nested.children[0].properties.contains_key(&ustr("Tags")));
}

#[test]
fn model_roundtrip_uses_mask_sidecar_and_keeps_other_defaults_shared() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{
        types::{Attributes, Tags},
        InstanceBuilder, WeakDom,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/Model.model.json"),
        r#"{"className":"Folder"}"#,
    )
    .unwrap();
    let path = dir.path().join("default.project.json");
    std::fs::write(&path, r#"{"name":"Root","syncRules":[{"pattern":"*.model.json","defaults":{"tags":["Shared"],"attributes":{"Owner":"shared","Other":true}}}],"tree":{"$path":"src"}}"#).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = build_project(&vfs, &project).unwrap();
    let mut tags = Tags::new();
    tags.push("Shared");
    let mut attrs = Attributes::new();
    attrs.insert("Other".into(), Variant::Bool(true));
    let new = WeakDom::new(
        InstanceBuilder::new("Folder").with_name("Root").with_child(
            InstanceBuilder::new("Folder")
                .with_name("Model")
                .with_property("Tags", tags)
                .with_property("Attributes", attrs)
                .with_child(InstanceBuilder::new("Folder").with_name("NewChild")),
        ),
    );
    let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
    changes.write_to_vfs(dir.path(), &vfs).unwrap();
    let model: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("src/Model.model.json")).unwrap())
            .unwrap();
    assert!(model.get("properties").is_none(), "{model}");
    assert!(model.get("attributes").is_none(), "{model}");
    let mask: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("src/Model.meta.json")).unwrap())
            .unwrap();
    assert_eq!(
        mask,
        json!({"ruleDefaults":{"disabledAttributes":["Owner"]}})
    );
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let tree = build_project(&vfs, &project).unwrap();
    let model = tree
        .inner()
        .get_by_ref(tree.inner().root().children()[0])
        .unwrap();
    let Variant::Attributes(attrs) = &model.properties[&ustr("Attributes")] else {
        panic!()
    };
    assert_eq!(attrs.get("Owner"), None);
    assert_eq!(attrs.get("Other"), Some(&Variant::Bool(true)));
    assert_eq!(model.children().len(), 1);
}

#[test]
fn binary_and_xml_roundtrips_preserve_rule_authority_or_refuse_lossy_columns() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{types::Tags, InstanceBuilder, WeakDom};
    for format in ["rbxm", "rbxmx"] {
        for child_tags in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir(dir.path().join("src")).unwrap();
            let source = dir.path().join(format!("src/Model.{format}"));
            let dom = WeakDom::new(
                InstanceBuilder::new("DataModel").with_child(
                    InstanceBuilder::new("Folder")
                        .with_name("Model")
                        .with_child(InstanceBuilder::new("Folder").with_name("Child")),
                ),
            );
            let mut bytes = Vec::new();
            if format == "rbxm" {
                rbx_binary::to_writer(&mut bytes, &dom, dom.root().children()).unwrap();
            } else {
                rbx_xml::to_writer(
                    &mut bytes,
                    &dom,
                    dom.root().children(),
                    rbx_xml::EncodeOptions::new(),
                )
                .unwrap();
            }
            std::fs::write(&source, &bytes).unwrap();
            let path = dir.path().join("default.project.json");
            std::fs::write(&path, serde_json::to_vec(&json!({"name":"Root","syncRules":[{"pattern":format!("*.{format}"),"defaults":{"tags":["Shared"]}}],"tree":{"$path":"src"}})).unwrap()).unwrap();
            let vfs = Vfs::new_default().unwrap();
            vfs.set_watch_enabled(false);
            let project = Project::load_exact(&vfs, &path, None).unwrap();
            let mut old = build_project(&vfs, &project).unwrap();
            let mut tags = Tags::new();
            tags.push("Shared");
            let mut child = InstanceBuilder::new("Folder")
                .with_name("Child")
                .with_property("Archivable", false);
            if child_tags {
                let mut own = Tags::new();
                own.push("Own");
                child = child.with_property("Tags", own);
            }
            let new = WeakDom::new(
                InstanceBuilder::new("Folder").with_name("Root").with_child(
                    InstanceBuilder::new("Folder")
                        .with_name("Model")
                        .with_property("Tags", tags)
                        .with_child(child),
                ),
            );
            let changes = syncback_loop(&vfs, &mut old, new, &project);
            if format == "rbxm" && child_tags {
                let error = match changes {
                    Err(error) => error,
                    Ok(_) => panic!("binary column authority loss was accepted"),
                };
                assert!(
                    format!("{error:#}").contains("materialize inherited rule default Tags"),
                    "{error:#}"
                );
                assert_eq!(std::fs::read(&source).unwrap(), bytes);
            } else {
                changes.unwrap().write_to_vfs(dir.path(), &vfs).unwrap();
                let project = Project::load_exact(&vfs, &path, None).unwrap();
                let after = build_project(&vfs, &project).unwrap();
                let model = after
                    .inner()
                    .get_by_ref(after.inner().root().children()[0])
                    .unwrap();
                let Variant::Tags(tags) = &model.properties[&ustr("Tags")] else {
                    panic!()
                };
                assert_eq!(tags.iter().collect::<Vec<_>>(), ["Shared"]);
            }
        }
    }
}

#[test]
fn jsonc_roundtrip_retains_explicit_empty_false_and_engine_default_overrides() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{
        types::{Attributes, Tags},
        InstanceBuilder, WeakDom,
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/Module.luau"), "return 1\n").unwrap();
    let meta_path = dir.path().join("src/Module.meta.jsonc");
    std::fs::write(&meta_path, r#"{"properties":{"Tags":[],"Archivable":true},"attributes":{"Owner":"explicit","Count":0},"ignoreUnknownInstances":false}"#).unwrap();
    let path = dir.path().join("default.project.json");
    std::fs::write(&path, r#"{"name":"Root","syncRules":[{"pattern":"*.luau","defaults":{"tags":["Shared"],"properties":{"Archivable":false},"attributes":{"Owner":"shared","Count":5,"Other":true},"ignoreUnknownInstances":true}}],"tree":{"$path":"src"}}"#).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = build_project(&vfs, &project).unwrap();
    let make_dom = || {
        let mut attrs = Attributes::new();
        attrs.insert("Owner".into(), Variant::String("explicit".into()));
        attrs.insert("Count".into(), Variant::Float64(0.0));
        attrs.insert("Other".into(), Variant::Bool(true));
        WeakDom::new(
            InstanceBuilder::new("Folder").with_name("Root").with_child(
                InstanceBuilder::new("ModuleScript")
                    .with_name("Module")
                    .with_property("Source", "return 2\n")
                    .with_property("Tags", Tags::new())
                    .with_property("Archivable", true)
                    .with_property("Attributes", attrs),
            ),
        )
    };
    let changes = syncback_loop(&vfs, &mut old, make_dom(), &project).unwrap();
    changes.write_to_vfs(dir.path(), &vfs).unwrap();
    assert!(!dir.path().join("src/Module.meta.json").exists());
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(meta_path).unwrap()).unwrap();
    assert_eq!(value["properties"]["Tags"], json!([]));
    assert_eq!(value["properties"]["Archivable"], true);
    assert_eq!(value["attributes"]["Owner"], "explicit");
    assert_eq!(value["attributes"]["Count"], 0.0);
    assert!(value["attributes"].get("Other").is_none());
    assert_eq!(value["ignoreUnknownInstances"], false);
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = build_project(&vfs, &project).unwrap();
    let repeated = syncback_loop(&vfs, &mut old, make_dom(), &project).unwrap();
    assert!(
        repeated.is_empty(),
        "second run generated {:?}",
        repeated.added_files()
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/Module.luau")).unwrap(),
        "return 2\n"
    );
}

#[test]
fn new_script_destinations_write_only_values_that_differ_from_their_rules() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{types::Tags, InstanceBuilder, WeakDom};
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    let path = dir.path().join("default.project.json");
    std::fs::write(&path, r#"{"name":"Root","syncRules":[{"pattern":"*.luau","defaults":{"tags":["Shared"],"attributes":{"Owner":"shared"}}}],"tree":{"$path":"src"}}"#).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = build_project(&vfs, &project).unwrap();
    let mut tags = Tags::new();
    tags.push("Shared");
    let new = WeakDom::new(
        InstanceBuilder::new("Folder").with_name("Root").with_child(
            InstanceBuilder::new("ModuleScript")
                .with_name("NewModule")
                .with_property("Source", "return 42")
                .with_property("Tags", tags),
        ),
    );
    let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
    changes.write_to_vfs(dir.path(), &vfs).unwrap();
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("src/NewModule.meta.json")).unwrap())
            .unwrap();
    assert_eq!(
        meta,
        json!({"ruleDefaults":{"disabledAttributes":["Owner"]}})
    );
}

fn roundtrip_project(explicit_binding: bool) -> serde_json::Value {
    json!({"name":"Root", "syncRules":[{"pattern":"*.luau", "defaults":{
        "tags":["Shared"],"attributes":{"Owner":"shared","Other":true},"ignoreUnknownInstances":true
    }}], "tree": if explicit_binding {
        json!({"$className":"Folder","Module":{"$path":"Module.luau"}})
    } else { json!({"$path":"src"}) }})
}

#[test]
fn syncback_omits_shared_values_and_persists_only_removed_attribute_masks() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{
        types::{Attributes, Tags},
        InstanceBuilder, WeakDom,
    };
    for explicit in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let base = if explicit {
            dir.path().to_owned()
        } else {
            dir.path().join("src")
        };
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("Module.luau"), "return 1").unwrap();
        let path = dir.path().join("default.project.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&roundtrip_project(explicit)).unwrap(),
        )
        .unwrap();
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
        let mut old = build_project(&vfs, &project).unwrap();
        let make_dom = |owner: bool| {
            let mut attrs = Attributes::new();
            if owner {
                attrs.insert("Owner".into(), Variant::String("shared".into()));
            }
            attrs.insert("Other".into(), Variant::Bool(true));
            let mut tags = Tags::new();
            tags.push("Shared");
            WeakDom::new(
                InstanceBuilder::new("Folder").with_name("Root").with_child(
                    InstanceBuilder::new("ModuleScript")
                        .with_name("Module")
                        .with_property("Source", "return 2")
                        .with_property("Tags", tags)
                        .with_property("Attributes", attrs),
                ),
            )
        };
        let changes = syncback_loop(&vfs, &mut old, make_dom(true), &project).unwrap();
        changes.write_to_vfs(dir.path(), &vfs).unwrap();
        assert!(
            !base.join("Module.meta.json").exists(),
            "shared values baked into metadata, explicit={explicit}"
        );
        let unchanged_project: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(unchanged_project, roundtrip_project(explicit));

        let project = Project::load_exact(&vfs, &path, None).unwrap();
        let mut old = build_project(&vfs, &project).unwrap();
        let changes = syncback_loop(&vfs, &mut old, make_dom(false), &project).unwrap();
        changes.write_to_vfs(dir.path(), &vfs).unwrap();
        let value: serde_json::Value = if explicit {
            let p: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            p["tree"]["Module"]["$ruleDefaults"].clone()
        } else {
            let p: serde_json::Value =
                serde_json::from_slice(&std::fs::read(base.join("Module.meta.json")).unwrap())
                    .unwrap();
            assert!(p.get("attributes").is_none());
            assert!(p.get("properties").is_none());
            p["ruleDefaults"].clone()
        };
        assert_eq!(value, json!({"disabledAttributes":["Owner"]}));
        let project = Project::load_exact(&vfs, &path, None).unwrap();
        let after = build_project(&vfs, &project).unwrap();
        let module = after
            .inner()
            .get_by_ref(after.inner().root().children()[0])
            .unwrap();
        let Variant::Attributes(attrs) = &module.properties[&ustr("Attributes")] else {
            panic!()
        };
        assert_eq!(attrs.get("Owner"), None);
        assert_eq!(attrs.get("Other"), Some(&Variant::Bool(true)));
    }
}
