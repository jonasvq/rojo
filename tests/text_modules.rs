use std::{fs, process::Command};

#[test]
fn csv_can_be_explicitly_synced_as_a_module_returning_its_text() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("items.csv"),
        "name,description\nSword,Sharp\n",
    )
    .unwrap();
    fs::write(dir.path().join("default.project.json"),
        r#"{"name":"Data","syncRules":[{"pattern":"*.csv","use":"textModule","suffix":".csv"}],"tree":{"$className":"Folder","Items":{"$path":"items.csv"}}}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "-o", "out.rbxmx"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(dir.path().join("out.rbxmx")).unwrap();
    let dom = rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new()).unwrap();
    let item = dom.descendants().find(|node| node.name == "Items").unwrap();
    assert_eq!(item.class, "ModuleScript");
    let rbx_dom_weak::types::Variant::String(source) = &item.properties[&"Source".into()] else {
        panic!("missing Source")
    };
    assert!(source.starts_with("return "));
    assert!(source.contains("name,description"));
}

#[test]
fn default_csv_and_text_mappings_are_unchanged_and_invalid_utf8_preserves_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/default.csv"),
        "Key,Source\nHello,World\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/default.txt"), "plain text").unwrap();
    fs::write(
        dir.path().join("default.project.json"),
        r#"{"name":"Root","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "-o", "out.rbxmx"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dom = rbx_xml::from_reader(
        fs::File::open(dir.path().join("out.rbxmx")).unwrap(),
        rbx_xml::DecodeOptions::new(),
    )
    .unwrap();
    assert!(dom
        .descendants()
        .any(|node| node.class == "LocalizationTable"));
    assert!(dom.descendants().any(|node| node.class == "StringValue"));
    let preserved = fs::read(dir.path().join("out.rbxmx")).unwrap();
    fs::write(dir.path().join("src/bad.bin"), [0xff, 0xfe]).unwrap();
    fs::write(dir.path().join("default.project.json"), r#"{"name":"Root","syncRules":[{"pattern":"**/*.bin","use":"textModule"}],"tree":{"$className":"Folder","$path":"src"}}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "-o", "out.rbxmx"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("bad.bin"));
    assert_eq!(fs::read(dir.path().join("out.rbxmx")).unwrap(), preserved);
}

#[test]
fn known_text_bindings_syncback_exact_bytes_without_lua_or_metadata_path_changes() {
    use librojo::{build_project, syncback_loop, Project};
    use memofs::Vfs;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    for explicit in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        let original = b"original\r\n";
        let input = dir.path().join("src/items.raw.csv");
        fs::write(&input, original).unwrap();
        fs::write(
            dir.path().join("src/items.meta.jsonc"),
            r#"{"id":"stable","attributes":{"Note":"original","Rojo_Id":"stable"}}"#,
        )
        .unwrap();
        let path = dir.path().join("default.project.json");
        let tree = if explicit {
            r#"{"$className":"Folder","Items":{"$path":"src/items.raw.csv"}}"#
        } else {
            r#"{"$className":"Folder","$path":"src"}"#
        };
        fs::write(&path, format!(r#"{{"name":"Root","syncRules":[{{"pattern":"**/*.raw.csv","use":"textModule","suffix":".raw.csv"}}],"tree":{tree}}}"#)).unwrap();
        for _ in 0..2 {
            let vfs = Vfs::new_default().unwrap();
            vfs.set_watch_enabled(false);
            let project = Project::load_exact(&vfs, &path, None).unwrap();
            let mut old = build_project(&vfs, &project).unwrap();
            let name = if explicit { "Items" } else { "items" };
            let target = rbx_dom_weak::types::Ref::new();
            let mut attributes = rbx_dom_weak::types::Attributes::new();
            attributes.insert("Note".into(), "updated".into());
            attributes.insert("Rojo_Id".into(), "stable".into());
            let mut root = InstanceBuilder::new("Folder").with_name("Root").with_child(
                InstanceBuilder::new("ModuleScript")
                    .with_name(name)
                    .with_referent(target)
                    .with_property("Attributes", attributes)
                    .with_property("Source", "return \"changed\\013\\010\\000123日本語\"\n"),
            );
            if !explicit {
                root = root.with_child(
                    InstanceBuilder::new("ObjectValue")
                        .with_name("Pointer")
                        .with_property("Value", target),
                );
            }
            let new = WeakDom::new(root);
            let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
            changes.write_to_vfs(dir.path(), &vfs).unwrap();
            assert_eq!(
                fs::read(&input).unwrap(),
                "changed\r\n\x00123日本語".as_bytes()
            );
            assert!(dir.path().join("src/items.meta.jsonc").exists());
            assert!(!dir.path().join("src/items.meta.json").exists());
            assert!(!dir.path().join("src/Items.meta.json").exists());
            assert_eq!(
                fs::read_dir(dir.path().join("src")).unwrap().count(),
                if explicit { 2 } else { 3 }
            );
            let fresh = Vfs::new_default().unwrap();
            fresh.set_watch_enabled(false);
            let reloaded = Project::load_exact(&fresh, &path, None).unwrap();
            let built = build_project(&fresh, &reloaded).unwrap();
            let instance = built
                .inner()
                .descendants()
                .find(|node| node.name == name)
                .unwrap();
            assert_eq!(instance.class, "ModuleScript");
            assert_eq!(
                built
                    .get_metadata(instance.referent())
                    .unwrap()
                    .specified_id
                    .as_ref()
                    .unwrap()
                    .as_str(),
                "stable"
            );
            let rbx_dom_weak::types::Variant::Attributes(attributes) =
                &instance.properties[&"Attributes".into()]
            else {
                panic!("missing attributes")
            };
            assert_eq!(
                attributes.get("Note"),
                Some(&rbx_dom_weak::types::Variant::String("updated".into()))
            );
            if !explicit {
                let pointer = built
                    .inner()
                    .descendants()
                    .find(|node| node.name == "Pointer")
                    .unwrap();
                assert_eq!(
                    pointer.properties.get(&"Value".into()),
                    Some(&rbx_dom_weak::types::Variant::Ref(instance.referent()))
                );
            }

            assert_eq!(
                instance.properties.get(&"Source".into()),
                Some(&rbx_dom_weak::types::Variant::String(
                    "return \"changed\\013\\010\\000123日本語\"\n".into()
                ))
            );
        }
    }
}

#[test]
fn syncback_rejects_code_class_changes_and_children_before_writing_any_file() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    let source_path = dir.path().join("src/items.csv");
    let project = dir.path().join("default.project.json");
    fs::write(&source_path, "keep original data\n").unwrap();
    let project_bytes = r#"{"name":"Root","syncRules":[{"pattern":"**/*.csv","use":"textModule","suffix":".csv"}],"tree":{"$className":"Folder","$path":"src"}}"#;
    fs::write(&project, project_bytes).unwrap();
    for (class, source, children) in [
        ("ModuleScript", "return os.time()", false),
        ("Script", "return \"data\"\n", false),
        ("ModuleScript", "return \"data\"\n", true),
    ] {
        let mut script = InstanceBuilder::new(class)
            .with_name("items")
            .with_property("Source", source);
        if children {
            script = script.with_child(InstanceBuilder::new("Folder").with_name("MustSurvive"));
        }
        let dom = WeakDom::new(
            InstanceBuilder::new("Folder")
                .with_name("Root")
                .with_child(script),
        );
        let file = fs::File::create(dir.path().join("input.rbxmx")).unwrap();
        rbx_xml::to_writer(file, &dom, &[dom.root_ref()], rbx_xml::EncodeOptions::new()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["syncback", "--input", "input.rbxmx", "-y"])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "accepted {class} {source} children={children}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("textModule"));
        assert_eq!(fs::read(&source_path).unwrap(), b"keep original data\n");
        assert_eq!(fs::read_to_string(&project).unwrap(), project_bytes);
        assert_eq!(fs::read_dir(dir.path().join("src")).unwrap().count(), 1);
    }
}

#[test]
fn live_text_module_updates_keep_identity_and_expose_original_file_paths() {
    use librojo::web_api::{ReadResponse, ServerInfoResponse};
    use rbx_dom_weak::types::Variant;
    use std::{
        net::TcpListener,
        process::Stdio,
        time::{Duration, Instant},
    };
    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/items.csv"), "first").unwrap();
    fs::write(dir.path().join("default.project.json"), r#"{"name":"Root","syncRules":[{"pattern":"**/*.csv","use":"textModule","suffix":".csv"}],"tree":{"$className":"Folder","$path":"src"}}"#).unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["serve", "--port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.path().join("stderr.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    let info: ServerInfoResponse = loop {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/api/rojo"))
            .send()
        {
            if response.status().is_success() {
                let raw: serde_json::Value =
                    rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
                break serde_json::from_value(raw).unwrap();
            }
        }
        assert!(Instant::now() < deadline);
        assert!(server.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut selected = None;
    for (file, expected) in [
        ("first", "return \"first\"\n"),
        ("second\nrow\x007", "return \"second\\010row\\0007\"\n"),
    ] {
        fs::write(dir.path().join("src/atomic.tmp"), file).unwrap();
        fs::rename(
            dir.path().join("src/atomic.tmp"),
            dir.path().join("src/items.csv"),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let response = client
                .get(format!(
                    "http://127.0.0.1:{port}/api/read/{}?showPaths=true",
                    info.root_instance_id
                ))
                .send()
                .unwrap();
            assert!(response.status().is_success());
            let bytes = response.bytes().unwrap();
            let raw: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
            let read: ReadResponse = serde_json::from_value(raw.clone()).unwrap();
            let item = read
                .instances
                .values()
                .find(|node| node.name == "items")
                .unwrap();
            assert_eq!(item.class_name, "ModuleScript");
            if let Some(id) = selected {
                assert_eq!(id, item.id);
            } else {
                selected = Some(item.id);
            }
            if item
                .properties
                .get(&"Source".into())
                .map(|value| value.as_ref())
                == Some(&Variant::String(expected.into()))
            {
                let raw: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
                let paths = raw["instances"][item.id.to_string()]["FilePaths"]
                    .as_array()
                    .unwrap();
                assert!(paths.iter().any(|path| path
                    .as_str()
                    .unwrap()
                    .replace('\\', "/")
                    .ends_with("src/items.csv")));
                break;
            }
            assert!(Instant::now() < deadline, "text module did not update");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["sourcemap"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["children"][0]["className"], "ModuleScript");
    assert!(json["children"][0]["filePaths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("src/items.csv")));
}
