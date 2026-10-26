use librojo::{snapshot_from_vfs, InstanceContext, InstanceSnapshot, PropertyAuthority};
use memofs::{InMemoryFs, Vfs, VfsSnapshot};
use serde_json::json;

fn fixture() -> Vfs {
    let mut fs = InMemoryFs::new();
    fs.load_snapshot("/project", VfsSnapshot::dir([
        ("default.project.json", VfsSnapshot::file(json!({"name":"Root","syncAuthority":[{"className":"Instance","properties":"studio"},{"className":"BasePart","properties":"twoWay"},{"className":"Part","properties":"filesystem"}],"tree":{"$className":"Folder","Model":{"$path":"Model.model.json"},"Inherited":{"$path":"inherited.project.json"},"Cleared":{"$path":"cleared.project.json"},"Replaced":{"$path":"replaced.project.json"}}}).to_string())),
        ("Model.model.json", VfsSnapshot::file(r#"{"className":"Model","children":[{"name":"Pole","className":"Part"},{"name":"Mesh","className":"MeshPart"}]}"#)),
        ("inherited.project.json", VfsSnapshot::file(r#"{"name":"Inherited","tree":{"$className":"Part"}}"#)),
        ("cleared.project.json", VfsSnapshot::file(r#"{"name":"Cleared","syncAuthority":[],"tree":{"$className":"Part"}}"#)),
        ("replaced.project.json", VfsSnapshot::file(r#"{"name":"Replaced","syncAuthority":[{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Part"}}"#)),
    ])).unwrap();
    let vfs = Vfs::new(fs);
    vfs.set_watch_enabled(false);
    vfs
}

fn snapshot(vfs: &Vfs, context: &InstanceContext, path: &str) -> InstanceSnapshot {
    snapshot_from_vfs(context, vfs, std::path::Path::new(path))
        .unwrap()
        .unwrap()
}

fn policy(node: &InstanceSnapshot) -> Option<PropertyAuthority> {
    node.metadata
        .effective_sync_authority
        .as_ref()
        .and_then(|authority| authority.resolve(node.class_name.as_str()).unwrap())
        .map(|policy| policy.properties)
}

#[test]
fn project_authority_inherits_and_replaces_across_embedded_and_nested_sources() {
    let root = snapshot(
        &fixture(),
        &InstanceContext::default(),
        "/project/default.project.json",
    );
    assert_eq!(policy(&root), Some(PropertyAuthority::Studio));
    let find = |name: &str| {
        root.children
            .iter()
            .find(|child| child.name == name)
            .unwrap()
    };
    assert_eq!(
        policy(find("Inherited")),
        Some(PropertyAuthority::Filesystem)
    );
    assert_eq!(policy(find("Cleared")), None);
    assert_eq!(policy(find("Replaced")), Some(PropertyAuthority::Studio));
    let model = find("Model");
    assert_eq!(policy(model), Some(PropertyAuthority::Studio));
    assert_eq!(
        policy(&model.children[0]),
        Some(PropertyAuthority::Filesystem)
    );
    assert_eq!(policy(&model.children[1]), Some(PropertyAuthority::TwoWay));
    assert!(root.metadata.context.sync_authority.is_none());
}

#[test]
fn removed_nested_override_reinherits_current_outer_authority_on_direct_refresh() {
    let vfs = fixture();
    let root = snapshot(
        &vfs,
        &InstanceContext::default(),
        "/project/default.project.json",
    );
    let node = root
        .children
        .iter()
        .find(|child| child.name == "Replaced")
        .unwrap();
    vfs.write(
        "/project/replaced.project.json",
        br#"{"name":"Replaced","tree":{"$className":"Part"}}"#,
    )
    .unwrap();
    let refreshed = snapshot(
        &vfs,
        &node.metadata.context,
        "/project/replaced.project.json",
    );
    assert_eq!(policy(&refreshed), Some(PropertyAuthority::Filesystem));
}

#[test]
fn project_authority_roundtrips_without_enabling_omitted_or_invalid_configuration() {
    let vfs = fixture();
    let path = std::path::Path::new("/project/default.project.json");
    let project = librojo::Project::load_exact(&vfs, path, None).unwrap();
    let value = serde_json::to_value(&project).unwrap();
    assert_eq!(value["syncAuthority"][0]["properties"], "studio");
    assert_eq!(value["syncAuthority"][0]["structure"], "filesystem");
    for authority in [
        json!(null),
        json!([{"className":"UnknownClass","properties":"twoWay"}]),
        json!([{"className":"Part","properties":"twoWay"},{"className":"Part","properties":"filesystem"}]),
        json!([{"className":"Part","properties":"twoWay","extra":true}]),
    ] {
        vfs.write(
            path,
            json!({"name":"Root","syncAuthority":authority,"tree":{"$className":"Folder"}})
                .to_string(),
        )
        .unwrap();
        assert!(librojo::Project::load_exact(&vfs, path, None).is_err());
    }
    for authority in [None, Some(json!([]))] {
        let mut value = json!({"name":"Root","tree":{"$className":"Folder"}});
        if let Some(authority) = authority {
            value["syncAuthority"] = authority;
        }
        vfs.write(path, value.to_string()).unwrap();
        let project = librojo::Project::load_exact(&vfs, path, None).unwrap();
        let encoded = serde_json::to_value(&project).unwrap();
        assert_eq!(value.get("syncAuthority"), encoded.get("syncAuthority"));
    }
}

#[test]
fn offline_syncback_retains_authority_configuration_and_jsonc_destination() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("default.project.jsonc");
    std::fs::write(&path, r#"// offline policy fixture
{"name":"Root","syncAuthority":[{"className":"Part","properties":"studio"}],"tree":{"$className":"Folder","Pole":{"$className":"Part","$properties":{"Transparency":0.1}}}}"#).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = librojo::build_project(&vfs, &project).unwrap();
    let new = WeakDom::new(
        InstanceBuilder::new("Folder").with_name("Root").with_child(
            InstanceBuilder::new("Part")
                .with_name("Pole")
                .with_property("Transparency", 0.8_f32),
        ),
    );
    let changes = librojo::syncback_loop(&vfs, &mut old, new, &project).unwrap();
    changes.write_to_vfs(dir.path(), &vfs).unwrap();
    let reloaded = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    assert_eq!(project.sync_authority, reloaded.sync_authority);
    assert!(!dir.path().join("default.project.json").exists());
    let rebuilt = librojo::build_project(&vfs, &reloaded).unwrap();
    let pole = rebuilt
        .inner()
        .get_by_ref(rebuilt.inner().root().children()[0])
        .unwrap();
    assert_eq!(
        pole.properties[&"Transparency".into()],
        rbx_dom_weak::types::Variant::Float32(0.8)
    );
}

#[test]
fn explicit_syncback_keeps_filesystem_script_properties_and_updates_studio_parts() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("default.project.json");
    let source = directory.path().join("script.luau");
    std::fs::write(&source, "return 'filesystem'\r\n").unwrap();
    std::fs::write(&path, json!({"name":"Root","syncAuthority":[{"className":"LuaSourceContainer","properties":"filesystem"},{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","Script":{"$path":"script.luau"},"Pole":{"$className":"Part","$properties":{"Transparency":0.1}}}}).to_string()).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = librojo::build_project(&vfs, &project).unwrap();
    let input = WeakDom::new(
        InstanceBuilder::new("Folder")
            .with_name("Root")
            .with_child(
                InstanceBuilder::new("ModuleScript")
                    .with_name("Script")
                    .with_property("Source", "return 'studio'")
                    .with_property("Archivable", false),
            )
            .with_child(
                InstanceBuilder::new("Part")
                    .with_name("Pole")
                    .with_property("Transparency", 0.8_f32),
            ),
    );
    let changes = librojo::syncback_loop(&vfs, &mut old, input, &project).unwrap();
    changes.write_to_vfs(directory.path(), &vfs).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"return 'filesystem'\r\n");
    let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    let rebuilt = librojo::build_project(&vfs, &project).unwrap();
    let pole = rebuilt
        .descendants(rebuilt.get_root_id())
        .find(|node| node.name() == "Pole")
        .unwrap();
    assert_eq!(
        pole.properties()[&"Transparency".into()],
        rbx_dom_weak::types::Variant::Float32(0.8)
    );
}

#[test]
fn filesystem_script_container_keeps_raw_source_while_studio_descendant_changes() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("default.project.json");
    std::fs::create_dir(directory.path().join("script")).unwrap();
    let source = directory.path().join("script/init.luau");
    std::fs::write(&source, "return 1\r\n").unwrap();
    std::fs::write(
        directory.path().join("script/Pole.model.json"),
        r#"{"className":"Part","properties":{"Transparency":0.1}}"#,
    )
    .unwrap();
    std::fs::write(&path, json!({"name":"Root","syncAuthority":[{"className":"LuaSourceContainer","properties":"filesystem"},{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","Script":{"$path":"script"}}}).to_string()).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = librojo::build_project(&vfs, &project).unwrap();
    let input = WeakDom::new(
        InstanceBuilder::new("Folder").with_name("Root").with_child(
            InstanceBuilder::new("ModuleScript")
                .with_name("Script")
                .with_property("Source", "return 2")
                .with_child(
                    InstanceBuilder::new("Part")
                        .with_name("Pole")
                        .with_property("Transparency", 0.8_f32),
                ),
        ),
    );
    let changes = librojo::syncback_loop(&vfs, &mut old, input, &project).unwrap();
    changes.write_to_vfs(directory.path(), &vfs).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"return 1\r\n");
    let rebuilt = librojo::build_project(
        &vfs,
        &librojo::Project::load_exact(&vfs, &path, None).unwrap(),
    )
    .unwrap();
    let pole = rebuilt
        .descendants(rebuilt.get_root_id())
        .find(|node| node.name() == "Pole")
        .unwrap();
    assert_eq!(
        pole.properties()[&"Transparency".into()],
        rbx_dom_weak::types::Variant::Float32(0.8)
    );
}

#[test]
fn explicit_syncback_keeps_project_root_label_with_filesystem_structure() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("default.project.json");
    std::fs::write(&path, json!({"name":"Root","syncAuthority":[{"className":"Folder","properties":"filesystem"},{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","Pole":{"$className":"Part","$properties":{"Transparency":0.1}}}}).to_string()).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = librojo::build_project(&vfs, &project).unwrap();
    let input = WeakDom::new(
        InstanceBuilder::new("Folder")
            .with_name("ExportedFileName")
            .with_child(
                InstanceBuilder::new("Part")
                    .with_name("Pole")
                    .with_property("Transparency", 0.8_f32),
            ),
    );
    librojo::syncback_loop(&vfs, &mut old, input, &project)
        .unwrap()
        .write_to_vfs(dir.path(), &vfs)
        .unwrap();
    let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    assert_eq!(project.name.as_deref(), Some("Root"));
    let rebuilt = librojo::build_project(&vfs, &project).unwrap();
    let pole = rebuilt
        .descendants(rebuilt.get_root_id())
        .find(|node| node.name() == "Pole")
        .unwrap();
    assert_eq!(
        pole.properties()[&"Transparency".into()],
        rbx_dom_weak::types::Variant::Float32(0.8)
    );
}

#[test]
fn embedded_models_apply_property_authority_without_excluding_studio_descendants() {
    use rbx_dom_weak::{types::Variant, InstanceBuilder, WeakDom};
    for format in ["model.json", "rbxmx", "rbxm"] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join(format!("Model.{format}"));
        let dom = WeakDom::new(
            InstanceBuilder::new("DataModel").with_child(
                InstanceBuilder::new("Folder")
                    .with_name("Model")
                    .with_property("Archivable", false)
                    .with_child(
                        InstanceBuilder::new("ModuleScript")
                            .with_name("Script")
                            .with_property("Source", "return 'disk'"),
                    )
                    .with_child(
                        InstanceBuilder::new("Part")
                            .with_name("Pole")
                            .with_property("Transparency", 0.1_f32),
                    ),
            ),
        );
        let mut bytes = Vec::new();
        match format {
            "model.json" => bytes = br#"{"className":"Folder","properties":{"Archivable":false},"children":[{"name":"Script","className":"ModuleScript","properties":{"Source":"return 'disk'"}},{"name":"Pole","className":"Part","properties":{"Transparency":0.1}}]}"#.to_vec(),
            "rbxmx" => rbx_xml::to_writer(&mut bytes, &dom, dom.root().children(), rbx_xml::EncodeOptions::new()).unwrap(),
            _ => rbx_binary::to_writer(&mut bytes, &dom, dom.root().children()).unwrap(),
        }
        std::fs::write(&source, bytes).unwrap();
        let path = directory.path().join("default.project.json");
        std::fs::write(&path, json!({"name":"Root","syncAuthority":[{"className":"Folder","properties":"filesystem"},{"className":"LuaSourceContainer","properties":"filesystem"},{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","Model":{"$path":format!("Model.{format}")}}}).to_string()).unwrap();
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
        let mut old = librojo::build_project(&vfs, &project).unwrap();
        for _ in 0..2 {
            let input = WeakDom::new(
                InstanceBuilder::new("Folder").with_name("Root").with_child(
                    InstanceBuilder::new("Folder")
                        .with_name("Model")
                        .with_property("Archivable", true)
                        .with_child(
                            InstanceBuilder::new("ModuleScript")
                                .with_name("Script")
                                .with_property("Source", "return 'studio'"),
                        )
                        .with_child(
                            InstanceBuilder::new("Part")
                                .with_name("Pole")
                                .with_property("Transparency", 0.8_f32),
                        ),
                ),
            );
            let changes = librojo::syncback_loop(&vfs, &mut old, input, &project).unwrap();
            changes.write_to_vfs(directory.path(), &vfs).unwrap();
            old = librojo::build_project(&vfs, &project).unwrap();
            let named = |name| {
                old.descendants(old.get_root_id())
                    .find(|node| node.name() == name)
                    .unwrap()
            };
            assert_eq!(
                named("Model").properties()[&"Archivable".into()],
                Variant::Bool(false),
                "{format}"
            );
            assert_eq!(
                named("Script").properties()[&"Source".into()],
                Variant::String("return 'disk'".into()),
                "{format}"
            );
            assert_eq!(
                named("Pole").properties()[&"Transparency".into()],
                Variant::Float32(0.8),
                "{format}"
            );
        }
    }
}

#[test]
fn syncback_normalization_cannot_remove_filesystem_owned_camera_structure() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("default.project.json");
    let original = json!({"name":"Root","syncAuthority":[{"className":"Camera","properties":"studio"}],"syncbackRules":{"syncCurrentCamera":false},"tree":{"$className":"DataModel","Workspace":{"$className":"Workspace","Camera":{"$className":"Camera"}}}}).to_string();
    std::fs::write(&path, &original).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = librojo::Project::load_exact(&vfs, &path, None).unwrap();
    let mut old = librojo::build_project(&vfs, &project).unwrap();
    let mut input = WeakDom::new(InstanceBuilder::new("DataModel").with_name("Root"));
    let workspace = input.insert(input.root_ref(), InstanceBuilder::new("Workspace"));
    let camera = input.insert(workspace, InstanceBuilder::new("Camera"));
    input
        .get_by_ref_mut(workspace)
        .unwrap()
        .properties
        .insert("CurrentCamera".into(), camera.into());
    let error = librojo::syncback_loop(&vfs, &mut old, input, &project)
        .err()
        .unwrap();
    assert!(
        format!("{error:#}").contains("syncAuthority filesystem structure"),
        "{error:#}"
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn explicit_syncback_respects_nested_empty_override_and_ignores_whole_excluded_classes() {
    use rbx_dom_weak::{types::Variant, InstanceBuilder, WeakDom};
    let vfs = fixture();
    let path = std::path::Path::new("/project/default.project.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(vfs.read(path).unwrap().as_slice()).unwrap();
    value["syncbackRules"] = json!({"ignoreClasses":["MeshPart"]});
    vfs.write(path, value.to_string()).unwrap();
    let project = librojo::Project::load_exact(&vfs, path, None).unwrap();
    let mut old = librojo::build_project(&vfs, &project).unwrap();
    let mut input = WeakDom::new(InstanceBuilder::new("Folder").with_name("Root"));
    let clones = old
        .inner()
        .clone_multiple_into_external(old.root().children(), &mut input);
    for id in clones {
        input.transfer_within(id, input.root_ref());
    }
    // Policy must come from each paired disk node, including explicit empty lists.
    for id in input
        .descendants()
        .map(|node| node.referent())
        .collect::<Vec<_>>()
    {
        let node = input.get_by_ref_mut(id).unwrap();
        if matches!(node.class.as_str(), "Part" | "MeshPart") {
            node.properties
                .insert("Transparency".into(), Variant::Float32(0.8));
        }
    }
    librojo::syncback_loop(&vfs, &mut old, input, &project)
        .unwrap()
        .write_to_vfs("/project", &vfs)
        .unwrap();
    let rebuilt = librojo::build_project(&vfs, &project).unwrap();
    for node in rebuilt
        .descendants(rebuilt.get_root_id())
        .filter(|node| matches!(node.class_name().as_str(), "Part" | "MeshPart"))
    {
        let expected = match node.name() {
            "Cleared" | "Replaced" => Some(&Variant::Float32(0.8)),
            _ => None,
        };
        assert_eq!(
            node.properties().get(&"Transparency".into()),
            expected,
            "{}",
            node.name()
        );
    }
}

#[test]
fn syncback_preserves_filesystem_reference_identity_or_refuses_before_writes() {
    use rbx_dom_weak::{
        types::{Attributes, Variant},
        InstanceBuilder, WeakDom,
    };
    for stable in [true, false] {
        let vfs = Vfs::new(InMemoryFs::new());
        vfs.set_watch_enabled(false);
        vfs.create_dir_all("/project").unwrap();
        let path = std::path::Path::new("/project/default.project.json");
        let original = json!({"name":"Root","syncAuthority":[{"className":"ObjectValue","properties":"filesystem"},{"className":"BasePart","properties":"studio"}],"tree":{"$className":"Folder","Target":{"$className":"Part","$id":"target"},"Other":{"$className":"Part"},"Link":{"$className":"ObjectValue","$attributes":{"Rojo_Target_Value":"target"}}}}).to_string();
        vfs.write(path, &original).unwrap();
        let project = librojo::Project::load_exact(&vfs, path, None).unwrap();
        let mut old = librojo::build_project(&vfs, &project).unwrap();
        let mut input = WeakDom::new(InstanceBuilder::new("Folder").with_name("Root"));
        let mut target = InstanceBuilder::new("Part").with_name("Target");
        if stable {
            target =
                target.with_property("Attributes", Attributes::new().with("Rojo_Id", "target"));
        }
        input.insert(input.root_ref(), target);
        let other = input.insert(
            input.root_ref(),
            InstanceBuilder::new("Part")
                .with_name("Other")
                .with_property("Transparency", 0.8_f32),
        );
        input.insert(
            input.root_ref(),
            InstanceBuilder::new("ObjectValue")
                .with_name("Link")
                .with_property("Value", other),
        );
        let result = librojo::syncback_loop(&vfs, &mut old, input, &project);
        if stable {
            result.unwrap().write_to_vfs("/project", &vfs).unwrap();
            let reloaded = librojo::Project::load_exact(&vfs, path, None).unwrap();
            let rebuilt = librojo::build_project(&vfs, &reloaded).unwrap();
            let named = |name| {
                rebuilt
                    .descendants(rebuilt.get_root_id())
                    .find(|node| node.name() == name)
                    .unwrap()
            };
            assert_eq!(
                named("Link").properties()[&"Value".into()],
                Variant::Ref(named("Target").id())
            );
            assert_eq!(
                named("Other").properties()[&"Transparency".into()],
                Variant::Float32(0.8)
            );
        } else {
            let error = result.err().unwrap();
            assert!(
                format!("{error:#}").contains("syncAuthority cannot preserve reference identity"),
                "{error:#}"
            );
            assert_eq!(vfs.read(path).unwrap().as_slice(), original.as_bytes());
        }
    }
}

#[test]
fn csv_authority_uses_common_boundary_without_changing_csv_middleware() {
    let vfs = fixture();
    vfs.write("/project/Table.csv", "Key,Source\nhello,world\n")
        .unwrap();
    for rules in [
        json!([]),
        json!([{"pattern":"*.csv","defaults":{"tags":["Shared"]}}]),
    ] {
        vfs.write("/project/default.project.json", json!({"name":"Root","syncAuthority":[{"className":"LocalizationTable","properties":"filesystem"}],"syncRules":rules,"tree":{"$className":"Folder","Table":{"$path":"Table.csv"}}}).to_string()).unwrap();
        let root = snapshot(
            &vfs,
            &InstanceContext::default(),
            "/project/default.project.json",
        );
        assert_eq!(
            policy(&root.children[0]),
            Some(PropertyAuthority::Filesystem)
        );
    }
}

#[cfg(feature = "cli")]
#[test]
fn offline_cli_build_and_stdin_do_not_require_live_authority_support() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let dir = tempfile::tempdir().unwrap();
    let project = br#"{"name":"Root","syncAuthority":[{"className":"Part","properties":"studio"}],"tree":{"$className":"Folder","Pole":{"$className":"Part"}}}"#;
    std::fs::write(dir.path().join("default.project.json"), project).unwrap();
    for input in ["default.project.json", "-"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", input, "-o", "out.rbxmx"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(project).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{input}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(std::fs::read_to_string(dir.path().join("out.rbxmx"))
            .unwrap()
            .contains("Pole"));
    }
}

#[cfg(feature = "cli")]
#[test]
fn offline_build_watch_reloads_policy_and_source_without_live_permissions() {
    use std::{
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    struct OwnedBuild(Child);
    impl Drop for OwnedBuild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let project_path = dir.path().join("default.project.json");
    let source = dir.path().join("Example.luau");
    let mut project = json!({"name":"Root","syncAuthority":[{"className":"LuaSourceContainer","properties":"studio"}],"tree":{"$className":"Folder","Example":{"$path":"Example.luau"}}});
    std::fs::write(&project_path, project.to_string()).unwrap();
    std::fs::write(&source, "return 'authority_watch_first'").unwrap();
    let mut build = OwnedBuild(
        Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", "--watch", "-o", "out.rbxmx"])
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(dir.path().join("build.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    let mut wait_for = |expected: &str| {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if std::fs::read_to_string(dir.path().join("out.rbxmx"))
                .is_ok_and(|output| output.contains(expected))
            {
                break;
            }
            assert!(
                build.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
                "{}",
                std::fs::read_to_string(dir.path().join("build.log")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for("authority_watch_first");
    project["syncAuthority"][0]["properties"] = "twoWay".into();
    std::fs::write(&project_path, project.to_string()).unwrap();
    std::fs::write(&source, "return 'authority_watch_second'").unwrap();
    wait_for("authority_watch_second");
}
