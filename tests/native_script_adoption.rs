use memofs::Vfs;
use rbx_dom_weak::WeakDom;

#[test]
fn syncback_script_roundtrip_preserves_class_and_run_context() {
    use librojo::{snapshot_from_vfs, syncback_loop, InstanceContext, Project, RojoTree};
    use rbx_dom_weak::{
        types::{Enum, Variant},
        InstanceBuilder,
    };

    let mut mismatches = Vec::new();
    for legacy in [false, true] {
        for (class, run_context) in [
            ("LocalScript", None),
            ("Script", Some(0)),
            ("Script", Some(1)),
            ("Script", Some(2)),
            ("Script", Some(3)),
            ("Script", Some(u32::MAX)),
            ("Script", None),
        ] {
            for with_child in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let src = dir.path().join("src");
                std::fs::create_dir(&src).unwrap();
                let path = dir.path().join("default.project.json");
                std::fs::write(
                    &path,
                    serde_json::to_vec(&serde_json::json!({
                        "name": "Root", "emitLegacyScripts": legacy, "tree": {"$path": src}
                    }))
                    .unwrap(),
                )
                .unwrap();
                let vfs = Vfs::new_default().unwrap();
                vfs.set_watch_enabled(false);
                let path = path.as_path();
                let project = Project::load_exact(&vfs, path, None).unwrap();
                let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                    .unwrap()
                    .unwrap();
                let mut old = RojoTree::new(snapshot);
                let make_new = |source: &str, run_context: Option<u32>| {
                    let mut script = InstanceBuilder::new(class)
                        .with_name("Example")
                        .with_property("Source", source);
                    if let Some(value) = run_context {
                        script = script.with_property("RunContext", Enum::from_u32(value));
                    }
                    if with_child {
                        script =
                            script.with_child(InstanceBuilder::new("Folder").with_name("Child"));
                    }
                    WeakDom::new(
                        InstanceBuilder::new("Folder")
                            .with_name("Root")
                            .with_child(script),
                    )
                };
                let new = make_new("return 42", run_context);
                let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
                changes.write_to_vfs(dir.path(), &vfs).unwrap();
                let after = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                    .unwrap()
                    .unwrap();
                let actual = &after.children[0];
                let case = format!(
                    "legacy={legacy}, class={class}, context={run_context:?}, child={with_child}"
                );
                if actual.class_name != class {
                    mismatches.push(format!("{case}: class became {}", actual.class_name));
                }
                assert_eq!(
                    actual.properties.get(&"Source".into()),
                    Some(&Variant::String("return 42".into())),
                    "{case}"
                );
                if let Some(value) = run_context {
                    let actual_context = actual
                        .properties
                        .get(&"RunContext".into())
                        .cloned()
                        .unwrap_or_else(|| Enum::from_u32(0).into());
                    if actual_context != Variant::Enum(Enum::from_u32(value)) {
                        mismatches.push(format!("{case}: RunContext became {actual_context:?}"));
                    }
                }
                if actual.children.len() != usize::from(with_child) {
                    mismatches.push(format!(
                        "{case}: children={}, paths={:?}",
                        actual.children.len(),
                        changes.added_paths()
                    ));
                }
                let mut rebuilt = RojoTree::new(after);
                let unchanged = syncback_loop(
                    &vfs,
                    &mut rebuilt,
                    make_new("return 42", run_context),
                    &project,
                )
                .unwrap();
                assert!(
                    unchanged.added_paths().is_empty() && unchanged.removed_paths().is_empty(),
                    "repeat {case}"
                );
                let edited_context =
                    (class == "Script").then_some(if run_context == Some(2) { 0 } else { 2 });
                let edited = syncback_loop(
                    &vfs,
                    &mut rebuilt,
                    make_new("return 43", edited_context),
                    &project,
                )
                .unwrap();
                edited.write_to_vfs(dir.path(), &vfs).unwrap();
                let after = snapshot_from_vfs(&InstanceContext::default(), &vfs, path)
                    .unwrap()
                    .unwrap();
                assert_eq!(after.children[0].class_name, class, "edited {case}");
                assert_eq!(
                    after.children[0].properties.get(&"Source".into()),
                    Some(&Variant::String("return 43".into())),
                    "edited {case}"
                );
                if let Some(value) = edited_context {
                    assert_eq!(
                        after.children[0]
                            .properties
                            .get(&"RunContext".into())
                            .cloned()
                            .unwrap_or_else(|| Enum::from_u32(0).into()),
                        Variant::Enum(Enum::from_u32(value)),
                        "edited {case}"
                    );
                }
                assert_eq!(after.children[0].children.len(), usize::from(with_child));
                let mut rebuilt = RojoTree::new(after);
                let repeat = syncback_loop(
                    &vfs,
                    &mut rebuilt,
                    make_new("return 43", edited_context),
                    &project,
                )
                .unwrap();
                assert!(
                    repeat.added_paths().is_empty() && repeat.removed_paths().is_empty(),
                    "edited repeat {case}"
                );
                assert!(
                    edited
                        .added_paths()
                        .iter()
                        .all(|path| !path.to_string_lossy().ends_with(".model.json")),
                    "JSON source fallback in {case}"
                );
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn native_lua_metadata_class_overrides_preserve_source_and_children() {
    use librojo::{snapshot_from_vfs, InstanceContext};
    use memofs::{InMemoryFs, VfsSnapshot};
    for directory in [false, true] {
        for (source_name, target, properties) in [
            ("main.client.luau", "LocalScript", "{}"),
            ("main.server.lua", "LocalScript", "{}"),
            ("main.client.lua", "Script", r#"{"RunContext":"Legacy"}"#),
        ] {
            let mut fs = InMemoryFs::new();
            let source = if directory {
                format!("/root/init{}", source_name.strip_prefix("main").unwrap())
            } else {
                format!("/root/{source_name}")
            };
            let meta = if directory {
                "/root/init.meta.json"
            } else {
                "/root/main.meta.json"
            };
            fs.load_snapshot(&source, VfsSnapshot::file("return 42"))
                .unwrap();
            fs.load_snapshot(
                meta,
                VfsSnapshot::file(format!(
                    r#"{{"className":"{target}","properties":{properties}}}"#
                )),
            )
            .unwrap();
            if directory {
                fs.load_snapshot("/root/Child", VfsSnapshot::empty_dir())
                    .unwrap();
            }
            // InMemoryFs requires directory snapshots to define directory listings.
            if directory {
                let init = source.rsplit('/').next().unwrap();
                fs.load_snapshot(
                    "/root",
                    VfsSnapshot::dir([
                        (init.to_owned(), VfsSnapshot::file("return 42")),
                        (
                            "init.meta.json".into(),
                            VfsSnapshot::file(format!(
                                r#"{{"className":"{target}","properties":{properties}}}"#
                            )),
                        ),
                        ("Child".into(), VfsSnapshot::empty_dir()),
                    ]),
                )
                .unwrap();
            }
            let vfs = Vfs::new(fs);
            let context = InstanceContext::with_emit_legacy_scripts(Some(false));
            let snapshot = snapshot_from_vfs(
                &context,
                &vfs,
                std::path::Path::new(if directory { "/root" } else { &source }),
            )
            .unwrap()
            .unwrap();
            assert_eq!(snapshot.class_name, target, "{source}");
            assert_eq!(
                snapshot.properties.get(&"Source".into()),
                Some(&rbx_dom_weak::types::Variant::String("return 42".into()))
            );
            if target == "LocalScript" {
                assert!(!snapshot.properties.contains_key(&"RunContext".into()));
            }
            assert_eq!(snapshot.children.len(), usize::from(directory));
        }
    }
}

fn input_script(class: &str, context: Option<u32>, with_child: bool, source: &str) -> WeakDom {
    use rbx_dom_weak::{types::Enum, InstanceBuilder};
    let mut script = InstanceBuilder::new(class)
        .with_name("Example")
        .with_property("Source", source);
    if let Some(value) = context {
        script = script.with_property("RunContext", Enum::from_u32(value));
    }
    if with_child {
        script = script.with_child(InstanceBuilder::new("Folder").with_name("Child"));
    }
    WeakDom::new(
        InstanceBuilder::new("Folder")
            .with_name("Root")
            .with_child(script),
    )
}

#[test]
fn gradual_rules_and_opposite_nested_emitters_roundtrip_natively() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{
        types::{Enum, Variant},
        InstanceBuilder,
    };
    for outer_legacy in [false, true] {
        for rule in [
            None,
            Some(("legacyClientScript", ".client.luau", "LocalScript", None)),
            Some(("runContextClientScript", ".server.luau", "Script", Some(2))),
            Some(("legacyServerScript", ".server.luau", "Script", Some(0))),
            Some(("runContextServerScript", ".server.luau", "Script", Some(1))),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let nested = temp.path().join("src/Nested");
            std::fs::create_dir_all(nested.join("scripts")).unwrap();
            let root_path = temp.path().join("default.project.json");
            std::fs::write(&root_path, serde_json::to_vec(&serde_json::json!({"name":"Root","emitLegacyScripts":outer_legacy,"tree":{"$path":"src"}})).unwrap()).unwrap();
            let mut nested_project = serde_json::json!({"name":"Nested","emitLegacyScripts":!outer_legacy,"tree":{"$path":"scripts"}});
            if let Some((middleware, suffix, _, _)) = rule {
                nested_project["syncRules"] =
                    serde_json::json!([{"pattern":"scripts/*","use":middleware,"suffix":suffix}]);
            }
            std::fs::write(
                nested.join("default.project.json"),
                serde_json::to_vec(&nested_project).unwrap(),
            )
            .unwrap();
            let vfs = Vfs::new_default().unwrap();
            vfs.set_watch_enabled(false);
            let project = Project::load_exact(&vfs, &root_path, None).unwrap();
            let mut old = build_project(&vfs, &project).unwrap();
            let (class, context) = rule
                .map(|(_, _, class, context)| (class, context))
                .unwrap_or(("LocalScript", None));
            let mut script = InstanceBuilder::new(class)
                .with_name("Example")
                .with_property("Source", "return 42");
            if let Some(context) = context {
                script = script.with_property("RunContext", Enum::from_u32(context));
            }
            let new = WeakDom::new(
                InstanceBuilder::new("Folder").with_name("Root").with_child(
                    InstanceBuilder::new("Folder")
                        .with_name("Nested")
                        .with_child(script),
                ),
            );
            let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
            changes.write_to_vfs(temp.path(), &vfs).unwrap();
            let after = build_project(&vfs, &project).unwrap();
            let parent = after.get_instance(after.root().children()[0]).unwrap();
            let script = after.get_instance(parent.children()[0]).unwrap();
            assert_eq!(
                script.class_name(),
                class,
                "outerLegacy={outer_legacy} rule={rule:?}"
            );
            if let Some(value) = context {
                assert_eq!(
                    script.properties().get(&"RunContext".into()),
                    Some(&Variant::Enum(Enum::from_u32(value)))
                );
            }
            assert!(changes
                .added_paths()
                .iter()
                .all(|path| !path.to_string_lossy().ends_with(".model.json")));
        }
    }
}

#[test]
fn invalid_metadata_overrides_are_contextual_and_read_only() {
    use librojo::{snapshot_from_vfs, InstanceContext};
    for source_name in [
        "Example.client.lua",
        "init.client.luau",
        "Example.lua",
        "init.lua",
    ] {
        for (class, properties) in [
            ("ModuleScript", "{}"),
            ("Part", "{}"),
            ("LocalScript", r#"{"RunContext":{"Enum":2}}"#),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let directory = source_name.starts_with("init.");
            let source = temp.path().join(source_name);
            let meta = temp.path().join(if directory {
                "init.meta.jsonc"
            } else {
                "Example.meta.jsonc"
            });
            std::fs::write(&source, "return 42").unwrap();
            let bytes = format!(r#"{{"className":"{class}","properties":{properties}}}"#);
            std::fs::write(&meta, &bytes).unwrap();
            let vfs = Vfs::new_default().unwrap();
            vfs.set_watch_enabled(false);
            let error = format!(
                "{:#}",
                snapshot_from_vfs(
                    &InstanceContext::with_emit_legacy_scripts(Some(false)),
                    &vfs,
                    if directory { temp.path() } else { &source }
                )
                .unwrap_err()
            );
            assert!(
                error.contains("className") || error.contains("RunContext"),
                "{error}"
            );
            assert!(error.contains("meta.jsonc"), "{error}");
            assert_eq!(std::fs::read_to_string(&meta).unwrap(), bytes);
            assert_eq!(std::fs::read_to_string(&source).unwrap(), "return 42");
        }
    }
}

#[test]
fn existing_jsonc_and_lua_names_survive_source_and_default_context_edits() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::types::{Enum, Variant};
    for directory in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        std::fs::create_dir(&src).unwrap();
        let folder = if directory {
            src.join("Example")
        } else {
            src.clone()
        };
        std::fs::create_dir_all(&folder).unwrap();
        let source = folder.join(if directory {
            "init.server.lua"
        } else {
            "Example.server.lua"
        });
        let meta = folder.join(if directory {
            "init.meta.jsonc"
        } else {
            "Example.meta.jsonc"
        });
        std::fs::write(&source, "return 1").unwrap();
        std::fs::write(
            &meta,
            r#"{"id":"native-script","properties":{"RunContext":"Server"}}"#,
        )
        .unwrap();
        let path = temp.path().join("default.project.json");
        std::fs::write(
            &path,
            r#"{"name":"Root","emitLegacyScripts":true,"tree":{"$path":"src"}}"#,
        )
        .unwrap();
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let project = Project::load_exact(&vfs, &path, None).unwrap();
        let mut old = build_project(&vfs, &project).unwrap();
        let changes = syncback_loop(
            &vfs,
            &mut old,
            input_script("Script", Some(0), false, "return 2"),
            &project,
        )
        .unwrap();
        changes.write_to_vfs(temp.path(), &vfs).unwrap();
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "return 2");
        assert!(!meta.with_extension("json").exists());
        assert!(!folder
            .join(if directory {
                "init.server.luau"
            } else {
                "Example.server.luau"
            })
            .exists());
        let after = build_project(&vfs, &project).unwrap();
        let script = after.get_instance(after.root().children()[0]).unwrap();
        assert_eq!(script.class_name(), "Script");
        assert_eq!(
            script
                .metadata()
                .specified_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("native-script")
        );
        assert_eq!(
            script.properties().get(&"RunContext".into()),
            Some(&Variant::Enum(Enum::from_u32(0)))
        );
    }
}

#[test]
fn unsupported_binding_and_source_layout_plans_leave_files_unchanged() {
    use librojo::{build_project, syncback_loop, Project};
    for case in [
        "children",
        "module",
        "duplicate-meta",
        "rename-rule",
        "incompatible-rule",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        std::fs::create_dir(&src).unwrap();
        let source = src.join(if case == "module" {
            "Example.luau"
        } else {
            "Example.server.luau"
        });
        std::fs::write(&source, "return 1").unwrap();
        if case == "duplicate-meta" {
            std::fs::write(src.join("Example.meta.json"), "{}").unwrap();
            std::fs::write(src.join("Example.meta.jsonc"), "{}").unwrap();
        }
        let mut config = serde_json::json!({"name":"Root","tree":{"$path":"src"}});
        if case == "rename-rule" {
            config["syncRules"] = serde_json::json!([{"pattern":"src/*","use":"serverScript"}]);
        }
        if case == "incompatible-rule" {
            config["syncRules"] = serde_json::json!([{"pattern":"src/*","use":"text"}]);
        }
        let path = temp.path().join("default.project.json");
        let project_bytes = serde_json::to_vec(&config).unwrap();
        std::fs::write(&path, &project_bytes).unwrap();
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let project = Project::load_exact(&vfs, &path, None).unwrap();
        let mut old = build_project(&vfs, &project).unwrap();
        let new = input_script("Script", Some(2), case == "children", "return 2");
        let error = syncback_loop(&vfs, &mut old, new, &project).err();
        assert!(error.is_some(), "unsupported case accepted: {case}");
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "return 1");
        assert_eq!(std::fs::read(&path).unwrap(), project_bytes);
    }
}

#[test]
fn project_bound_native_script_preserves_engine_default_context_in_project_properties() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::types::{Enum, Variant};
    for directory in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let source = if directory {
            let folder = temp.path().join("script");
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("init.server.luau"), "return 1").unwrap();
            folder
        } else {
            let file = temp.path().join("Example.server.lua");
            std::fs::write(&file, "return 1").unwrap();
            file
        };
        let path = temp.path().join("default.project.json");
        std::fs::write(&path, serde_json::to_vec(&serde_json::json!({"name":"Root","emitLegacyScripts":false,"tree":{"$className":"Folder","Example":{"$path":source}}})).unwrap()).unwrap();
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let project = Project::load_exact(&vfs, &path, None).unwrap();
        let mut old = build_project(&vfs, &project).unwrap();
        let changes = syncback_loop(
            &vfs,
            &mut old,
            input_script("Script", Some(0), false, "return 2"),
            &project,
        )
        .unwrap();
        changes.write_to_vfs(temp.path(), &vfs).unwrap();
        let loaded = Project::load_exact(&vfs, &path, None).unwrap();
        assert_eq!(
            loaded.tree.children["Example"].path,
            project.tree.children["Example"].path
        );
        let after = build_project(&vfs, &loaded).unwrap();
        let script = after.get_instance(after.root().children()[0]).unwrap();
        assert_eq!(script.class_name(), "Script");
        assert_eq!(
            script.properties().get(&"Source".into()),
            Some(&Variant::String("return 2".into()))
        );
        assert_eq!(
            script.properties().get(&"RunContext".into()),
            Some(&Variant::Enum(Enum::from_u32(0)))
        );
    }
}
