use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use librojo::{snapshot_from_vfs, InstanceContext};
use memofs::Vfs;
use serde_json::json;

fn build(path: &Path, output: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("build")
        .arg(path)
        .arg("-o")
        .arg(output)
        .output()
        .unwrap()
}

fn fixture(dir: &Path, name: &str, version: Option<u32>) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/Client.client.luau"), "return 1").unwrap();
    let mut config = json!({"tree":{"$className":"Folder","$path":"src"}});
    if let Some(version) = version {
        config["$version"] = version.into();
    }
    fs::write(dir.join(name), serde_json::to_vec(&config).unwrap()).unwrap();
}

#[test]
fn v2_init_aliases_support_directory_explicit_file_stem_and_folder_name() {
    for name in ["init.project.json", "init.project.jsonc"] {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), name, Some(2));
        for path in [
            dir.path().to_owned(),
            dir.path().join(name),
            dir.path().join("init"),
        ] {
            let result = build(&path, &dir.path().join("out.rbxmx"));
            assert!(
                result.status.success(),
                "{path:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let dom = rbx_xml::from_reader(
                fs::File::open(dir.path().join("out.rbxmx")).unwrap(),
                rbx_xml::DecodeOptions::new(),
            )
            .unwrap();
            let root = dom.get_by_ref(dom.root().children()[0]).unwrap();
            assert_eq!(root.name, dir.path().file_name().unwrap().to_str().unwrap());
            assert!(dom
                .descendants()
                .any(|node| node.name == "Client" && node.class == "Script"));
        }
    }
}

#[test]
fn init_v2_conflicts_are_reported_with_both_paths_and_explicit_files_work() {
    for conflict in [
        "default.project.json",
        "default.project.jsonc",
        "init.project.jsonc",
        "init.lua",
        "init.csv",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), "init.project.json", Some(2));
        let content = if conflict.ends_with("json") || conflict.ends_with("jsonc") {
            r#"{"$version":2,"name":"Other","tree":{"$className":"Folder"}}"#
        } else if conflict.ends_with("csv") {
            "Key,en\nhello,world"
        } else {
            "return 1"
        };
        fs::write(dir.path().join(conflict), content).unwrap();
        let result = build(dir.path(), &dir.path().join("out.rbxmx"));
        assert!(!result.status.success(), "accepted conflict {conflict}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains("init.project.json") && error.contains(conflict),
            "{error}"
        );
        assert!(build(
            &dir.path().join("init.project.json"),
            &dir.path().join("out.rbxmx")
        )
        .status
        .success());
    }
}

#[test]
fn unversioned_and_v1_init_projects_remain_named_children() {
    for version in [None, Some(1)] {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), "init.project.json", version);
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        let root = snapshot_from_vfs(&InstanceContext::new(), &vfs, dir.path())
            .unwrap()
            .unwrap();
        assert!(root.children.iter().any(|node| node.name == "init"));
        assert!(!build(dir.path(), &dir.path().join("out.rbxmx"))
            .status
            .success());
        let explicit = build(
            &dir.path().join("init.project.json"),
            &dir.path().join("out.rbxmx"),
        );
        assert!(
            explicit.status.success(),
            "{}",
            String::from_utf8_lossy(&explicit.stderr)
        );
    }
}

#[test]
fn shared_vfs_discovery_checks_bom_jsonc_nested_names_cycles_and_legacy_priority() {
    use librojo::Project;
    use memofs::{InMemoryFs, VfsSnapshot};
    let mut fs = InMemoryFs::new();
    fs.load_snapshot("/Outer", VfsSnapshot::dir([
        ("default.project.json", VfsSnapshot::file(r#"{"name":"Outer","tree":{"$path":"Nested"}}"#)),
        ("Nested", VfsSnapshot::dir([
            ("init.project.jsonc", VfsSnapshot::file("\u{feff}{\"$version\":2,/*entry*/\"tree\":{\"$className\":\"Folder\",\"Leaf\":{\"$className\":\"Folder\"}},}")),
        ])),
    ])).unwrap();
    let vfs = Vfs::new(fs);
    let project = Project::load_fuzzy(&vfs, Path::new("/Outer/Nested"))
        .unwrap()
        .unwrap();
    assert_eq!(project.name.as_deref(), Some("Nested"));
    assert_eq!(
        project.file_location,
        Path::new("/Outer/Nested/init.project.jsonc")
    );
    let snapshot = snapshot_from_vfs(&InstanceContext::new(), &vfs, Path::new("/Outer"))
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.children[0].name, "Leaf");
    vfs.write(
        "/Outer/Nested/init.project.jsonc",
        br#"{"$version":2,"tree":{"$path":"."}}"#,
    )
    .unwrap();
    let error = snapshot_from_vfs(&InstanceContext::new(), &vfs, Path::new("/Outer")).unwrap_err();
    assert!(
        format!("{error:#}").contains("Circular project reference"),
        "{error:#}"
    );
    vfs.write("/Outer/Nested/init.project.jsonc", b"malformed")
        .unwrap();
    let error = Project::load_fuzzy(&vfs, Path::new("/Outer/Nested")).unwrap_err();
    assert!(error.to_string().contains("init.project.jsonc"));
    vfs.write(
        "/Outer/Nested/init.project.jsonc",
        br#"{"$version":1,"tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    for (name, root) in [
        ("default.project.jsonc", "Jsonc"),
        ("default.project.json", "Json"),
    ] {
        vfs.write(
            Path::new("/Outer/Nested").join(name),
            serde_json::to_vec(&json!({"name":root,"tree":{"$className":"Folder"}})).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(
        Project::load_fuzzy(&vfs, Path::new("/Outer/Nested"))
            .unwrap()
            .unwrap()
            .name
            .as_deref(),
        Some("Json")
    );
    // Legacy default entries still win over native init files, as before.
    vfs.write("/Outer/Nested/init.lua", b"return 1").unwrap();
    let root = snapshot_from_vfs(&InstanceContext::new(), &vfs, Path::new("/Outer/Nested"))
        .unwrap()
        .unwrap();
    assert_eq!(root.name, "Json");
    assert_eq!(root.class_name, "Folder");
}

#[test]
fn nested_entry_syncback_and_fmt_keep_the_actual_alias_path() {
    use librojo::{build_project, syncback_loop, Project};
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    for explicit in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("containers/Entry");
        fs::create_dir_all(&entry).unwrap();
        fixture(&entry, "init.project.jsonc", Some(2));
        fs::write(
        dir.path().join("default.project.json"),
        serde_json::to_vec(&if explicit {
            json!({"name":"Outer","tree":{"$className":"Folder","Entry":{"$path":"containers/Entry"}}})
        } else {
            json!({"name":"Outer","tree":{"$path":"containers"}})
        }).unwrap(),
    )
    .unwrap();
        let vfs = Vfs::new_default().unwrap();
        vfs.set_watch_enabled(false);
        for source in ["return 10", "return 11"] {
            let project = Project::load_fuzzy(&vfs, dir.path()).unwrap().unwrap();
            let mut old = build_project(&vfs, &project).unwrap();
            let new = WeakDom::new(
                InstanceBuilder::new("Folder")
                    .with_name("Outer")
                    .with_child(
                        InstanceBuilder::new("Folder")
                            .with_name("Entry")
                            .with_child(
                                InstanceBuilder::new("Script")
                                    .with_name("Client")
                                    .with_property("Source", source)
                                    .with_property(
                                        "RunContext",
                                        rbx_dom_weak::types::Enum::from_u32(2),
                                    ),
                            )
                            .with_child(InstanceBuilder::new("Folder").with_name("Added")),
                    ),
            );
            let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
            changes.write_to_vfs(dir.path(), &vfs).unwrap();
            let after = build_project(&vfs, &project).unwrap();
            assert!(after
                .descendants(after.get_root_id())
                .any(|node| node.name() == "Added"));
            let source_text = fs::read_to_string(entry.join("src/Client.client.luau")).unwrap();
            assert_eq!(source_text, source);
            assert!(!entry.join("default.project.json").exists());
            let alias = Project::load_exact(&vfs, &entry.join("init.project.jsonc"), None).unwrap();
            assert_eq!(
                alias.format_version,
                Some(librojo::ProjectFormatVersion::V2)
            );
        }
        let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .arg("fmt-project")
            .arg(&entry)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let formatted: serde_json::Value =
            serde_json::from_slice(&fs::read(entry.join("init.project.jsonc")).unwrap()).unwrap();
        assert_eq!(formatted["$version"], 2);
        let map_path = dir.path().join("map.json");
        let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .arg("sourcemap")
            .arg(&entry)
            .arg("--output")
            .arg(&map_path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let map = fs::read_to_string(map_path).unwrap();
        assert!(
            map.contains("init.project.jsonc") && map.contains("Client.client.luau"),
            "{map}"
        );
    }
}

#[test]
fn native_watch_reselects_aliases_after_version_rename_remove_and_conflict_recovery() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    struct Watch(std::process::Child);
    impl Drop for Watch {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("Entry");
    fs::create_dir(&entry).unwrap();
    fixture(&entry, "init.project.json", Some(2));
    let output_path = dir.path().join("out.rbxmx");
    let error_path = dir.path().join("watch.log");
    let mut watcher = Watch(
        Command::new(env!("CARGO_BIN_EXE_rojo"))
            .arg("build")
            .arg(&entry)
            .args(["--watch", "-o"])
            .arg(&output_path)
            .stdout(Stdio::null())
            .stderr(fs::File::create(&error_path).unwrap())
            .spawn()
            .unwrap(),
    );
    let wait = |stage: &str, active_alias: bool, source: &str| {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Ok(bytes) = fs::read(&output_path) {
                if let Ok(dom) =
                    rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new())
                {
                    let root = dom.get_by_ref(dom.root().children()[0]).unwrap();
                    let direct_client = root
                        .children()
                        .iter()
                        .any(|id| dom.get_by_ref(*id).unwrap().name == "Client");
                    let correct_source = dom
                        .descendants()
                        .filter(|node| node.name == "Client")
                        .all(|node| {
                            node.properties.get(&"Source".into())
                                == Some(&rbx_dom_weak::types::Variant::String(source.into()))
                        });
                    if direct_client == active_alias && correct_source {
                        assert!(dom.descendants().any(|node| node.name == "Client"));
                        break;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "stage={stage}, alias={active_alias}, source={source}: {}",
                fs::read_to_string(&error_path).unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait("initial", true, "return 1");
    fixture(&entry, "init.project.json", Some(1));
    wait("V1", false, "return 1");
    fixture(&entry, "init.project.json", Some(2));
    wait("V2 again", true, "return 1");
    fs::rename(
        entry.join("init.project.json"),
        entry.join("init.project.jsonc"),
    )
    .unwrap();
    fs::write(entry.join("src/Client.client.luau"), "return 2").unwrap();
    wait("renamed", true, "return 2");
    fs::remove_file(entry.join("init.project.jsonc")).unwrap();
    wait("removed", false, "return 2");
    fixture(&entry, "init.project.jsonc", Some(2));
    wait("recreated", true, "return 1");
    fs::write(entry.join("init.lua"), "return 3").unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let errors = fs::read_to_string(&error_path).unwrap();
        if errors.contains("Ambiguous project directory entries") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "conflict was not observed: {errors}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::remove_file(entry.join("init.lua")).unwrap();
    fs::write(entry.join("src/Client.client.luau"), "return 99").unwrap();
    wait("recovered", true, "return 99");
    assert!(watcher.0.try_wait().unwrap().is_none());
    // Compare the final output to a completely new VFS snapshot.
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let fresh = snapshot_from_vfs(&InstanceContext::new(), &vfs, &entry)
        .unwrap()
        .unwrap();
    assert_eq!(fresh.children.len(), 1);
    assert_eq!(fresh.children[0].name, "Client");
    assert_eq!(
        fresh.children[0].properties.get(&"Source".into()),
        Some(&rbx_dom_weak::types::Variant::String("return 99".into()))
    );
}
