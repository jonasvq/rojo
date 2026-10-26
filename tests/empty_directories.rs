use std::{fs, process::Command};

#[test]
fn opt_in_ignores_implicit_empty_directories_and_keeps_the_explicit_root() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/LeftBehind/Nested")).unwrap();
    fs::create_dir_all(dir.path().join("src/Intentional")).unwrap();
    fs::write(dir.path().join("src/Intentional/.gitkeep"), "").unwrap();
    fs::write(dir.path().join("src/Live.lua"), "return 1").unwrap();
    fs::write(dir.path().join("default.project.json"),
        r#"{"name":"Root","ignoreEmptyDirectories":true,"tree":{"$className":"Folder","$path":"src"}}"#).unwrap();
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
    assert!(!dom.descendants().any(|node| node.name == "LeftBehind"));
    assert!(dom.descendants().any(|node| node.name == "Intentional"));
    assert!(dom.descendants().any(|node| node.name == "Root"));
    assert!(dom.descendants().any(|node| node.name == "Live"));
}

fn build(dir: &std::path::Path) -> rbx_dom_weak::WeakDom {
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir)
        .args(["build", "-o", "out.rbxmx"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(dir.join("out.rbxmx")).unwrap();
    rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new()).unwrap()
}

#[test]
fn omission_false_and_explicit_bindings_preserve_empty_folders() {
    for option in [
        "",
        "\"ignoreEmptyDirectories\":false,",
        "\"ignoreEmptyDirectories\":true,",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/Implicit")).unwrap();
        fs::create_dir(dir.path().join("Explicit")).unwrap();
        fs::write(dir.path().join("default.project.json"), format!(r#"{{{option}"name":"Root","tree":{{"$className":"Folder","$path":"src","Pinned":{{"$path":"../Explicit"}},"Authored":{{"$className":"Folder"}}}}}}"#).replace("../Explicit", "Explicit")).unwrap();
        let dom = build(dir.path());
        assert!(dom.descendants().any(|node| node.name == "Root"));
        assert!(dom.descendants().any(|node| node.name == "Pinned"));
        assert!(dom.descendants().any(|node| node.name == "Authored"));
        assert_eq!(
            dom.descendants().any(|node| node.name == "Implicit"),
            !option.contains("true")
        );
    }
}

#[test]
fn every_physical_file_and_init_representation_preserves_its_container() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "Unknown",
        "Ignored",
        "Metadata",
        "Script",
        "IgnoredSubtree/Inner",
    ] {
        fs::create_dir_all(dir.path().join("src").join(name)).unwrap();
    }
    fs::write(
        dir.path().join("src/Unknown/data.unrecognized"),
        "ignored by default middleware",
    )
    .unwrap();
    fs::write(dir.path().join("src/Ignored/skip.lua"), "return 1").unwrap();
    fs::write(
        dir.path().join("src/Metadata/init.meta.json"),
        r#"{"className":"Configuration"}"#,
    )
    .unwrap();
    fs::write(dir.path().join("src/Script/init.lua"), "return 2").unwrap();
    fs::write(
        dir.path().join("src/IgnoredSubtree/Inner/payload.bin"),
        "unrecognized data",
    )
    .unwrap();
    fs::write(dir.path().join("default.project.json"), r#"{"name":"Root","ignoreEmptyDirectories":true,"globIgnorePaths":["src/Ignored/skip.lua","src/IgnoredSubtree/Inner"],"tree":{"$className":"Folder","$path":"src"}}"#).unwrap();
    let dom = build(dir.path());
    for name in ["Unknown", "Ignored", "Metadata", "Script", "IgnoredSubtree"] {
        assert!(
            dom.descendants().any(|node| node.name == name),
            "lost {name}"
        );
    }
    assert!(!dom.descendants().any(|node| node.name == "skip"));
    assert_eq!(
        dom.descendants()
            .find(|node| node.name == "Metadata")
            .unwrap()
            .class,
        "Configuration"
    );
    assert_eq!(
        dom.descendants()
            .find(|node| node.name == "Script")
            .unwrap()
            .class,
        "ModuleScript"
    );
    fs::write(dir.path().join("src/Metadata/init.meta.json"), "{invalid").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "-o", "out.rbxmx"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("init.meta.json"));
}

#[test]
fn nested_projects_inherit_and_explicit_false_or_true_override() {
    let dir = tempfile::tempdir().unwrap();
    for nested in ["Inherited", "Preserved", "Filtered"] {
        fs::create_dir_all(dir.path().join("src").join(nested).join("children/Empty")).unwrap();
        let option = match nested {
            "Preserved" => "\"ignoreEmptyDirectories\":false,",
            "Filtered" => "\"ignoreEmptyDirectories\":true,",
            _ => "",
        };
        fs::write(dir.path().join("src").join(nested).join("default.project.json"), format!(r#"{{{option}"name":"{nested}","tree":{{"$className":"Folder","$path":"children"}}}}"#)).unwrap();
    }
    for outer in [true, false] {
        fs::write(dir.path().join("default.project.json"), format!(r#"{{"name":"Root","ignoreEmptyDirectories":{outer},"tree":{{"$className":"Folder","$path":"src"}}}}"#)).unwrap();
        let dom = build(dir.path());
        for (name, empty) in [
            ("Inherited", !outer),
            ("Preserved", true),
            ("Filtered", false),
        ] {
            let node = dom.descendants().find(|node| node.name == name).unwrap();
            assert_eq!(!node.children().is_empty(), empty, "{name} outer {outer}");
        }
    }
}

#[test]
fn sourcemap_and_syncback_keep_intentional_empty_folder_markers() {
    use librojo::{build_project, syncback_loop, Project};
    use memofs::Vfs;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/LeftBehind")).unwrap();
    let path = dir.path().join("default.project.json");
    fs::write(&path, r#"{"name":"Root","ignoreEmptyDirectories":true,"tree":{"$className":"Folder","$path":"src"}}"#).unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let mut tree = build_project(&vfs, &project).unwrap();
    let new = WeakDom::new(
        InstanceBuilder::new("Folder")
            .with_name("Root")
            .with_child(InstanceBuilder::new("Folder").with_name("Intentional")),
    );
    let snapshot = syncback_loop(&vfs, &mut tree, new, &project).unwrap();
    assert!(snapshot
        .added_files()
        .iter()
        .any(|path| path.ends_with("Intentional/.gitkeep")));
    fs::create_dir(dir.path().join("src/Intentional")).unwrap();
    fs::write(dir.path().join("src/Intentional/.gitkeep"), "").unwrap();
    let dom = build(dir.path());
    assert!(dom.descendants().any(|node| node.name == "Intentional"));
    assert!(!dom.descendants().any(|node| node.name == "LeftBehind"));
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["sourcemap", "--include-non-scripts"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let children = json["children"].as_array().unwrap();
    assert!(children.iter().any(|child| child["name"] == "Intentional"));
    assert!(!children.iter().any(|child| child["name"] == "LeftBehind"));
}

#[test]
fn watch_converges_when_empty_ancestors_gain_lose_and_move_their_last_file() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    struct Watching(std::process::Child);
    impl Drop for Watching {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/Branch/Nested")).unwrap();
    fs::write(dir.path().join("default.project.json"), r#"{"name":"Root","ignoreEmptyDirectories":true,"tree":{"$className":"Folder","$path":"src"}}"#).unwrap();
    let mut child = Watching(
        Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", "--watch", "-o", "out.rbxmx"])
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.path().join("stderr.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    let wait_for = |branch: Option<&str>, source: Option<&str>| {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Ok(bytes) = fs::read(dir.path().join("out.rbxmx")) {
                if let Ok(dom) =
                    rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new())
                {
                    let nodes: Vec<_> = dom.descendants().collect();
                    let valid = nodes.iter().any(|node| node.name == "Root")
                        && ["Branch", "Moved"].into_iter().all(|name| {
                            nodes.iter().any(|node| node.name == name) == (branch == Some(name))
                        })
                        && source.is_none_or(|source| {
                            nodes.iter().any(|node| {
                                node.properties.get(&"Source".into())
                                    == Some(&rbx_dom_weak::types::Variant::String(source.into()))
                            })
                        });
                    if valid {
                        break;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "watch did not converge: {}",
                fs::read_to_string(dir.path().join("stderr.log")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for(None, None);
    fs::write(dir.path().join("src/Branch/Nested/Last.lua"), "return 1").unwrap();
    wait_for(Some("Branch"), Some("return 1"));
    fs::remove_file(dir.path().join("src/Branch/Nested/Last.lua")).unwrap();
    wait_for(None, None);
    fs::write(dir.path().join("src/Branch/Nested/Last.lua"), "return 2").unwrap();
    wait_for(Some("Branch"), Some("return 2"));
    fs::rename(dir.path().join("src/Branch"), dir.path().join("src/Moved")).unwrap();
    wait_for(Some("Moved"), Some("return 2"));
    fs::remove_file(dir.path().join("src/Moved/Nested/Last.lua")).unwrap();
    wait_for(None, None);
    fs::write(dir.path().join("src/Moved/Nested/Last.lua"), "return 3").unwrap();
    wait_for(Some("Moved"), Some("return 3"));
    assert!(child.0.try_wait().unwrap().is_none());
}

#[test]
fn live_api_retains_root_identity_and_reloads_the_filter_setting() {
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
    fs::create_dir_all(dir.path().join("src/Empty")).unwrap();
    fs::create_dir_all(dir.path().join("src/Override/children/NestedEmpty")).unwrap();
    let nested_project = dir.path().join("src/Override/default.project.json");
    fs::write(&nested_project, r#"{"name":"Override","ignoreEmptyDirectories":false,"tree":{"$className":"Folder","$path":"children"}}"#).unwrap();
    let project = dir.path().join("default.project.json");
    let set_option = |enabled| {
        fs::write(&project, format!(r#"{{"name":"Root","ignoreEmptyDirectories":{enabled},"tree":{{"$className":"Folder","$path":"src"}}}}"#)).unwrap()
    };
    set_option(true);
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
    let root = loop {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/api/rojo"))
            .send()
        {
            if response.status().is_success() {
                let info: serde_json::Value =
                    rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
                break info["rootInstanceId"].as_str().unwrap().to_owned();
            }
        }
        assert!(Instant::now() < deadline);
        assert!(server.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    };
    let wait_for = |folder: bool, leaf: bool| {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let response = client
                .get(format!("http://127.0.0.1:{port}/api/read/{root}"))
                .send()
                .unwrap();
            assert!(response.status().is_success());
            let body: serde_json::Value =
                rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
            let instances = body["instances"].as_object().unwrap();
            assert_eq!(instances[&root]["Name"], "Root");
            if instances.values().any(|node| node["Name"] == "Empty") == folder
                && instances.values().any(|node| node["Name"] == "Leaf") == leaf
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "live filter did not converge: {body}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for(false, false);
    fs::write(dir.path().join("src/Empty/Leaf.lua"), "return 1").unwrap();
    wait_for(true, true);
    fs::remove_file(dir.path().join("src/Empty/Leaf.lua")).unwrap();
    wait_for(false, false);
    set_option(false);
    wait_for(true, false);
    set_option(true);
    wait_for(false, false);
    for (option, expected) in [
        ("", false),
        ("\"ignoreEmptyDirectories\":false,", true),
        ("", false),
    ] {
        fs::write(&nested_project, format!(r#"{{{option}"name":"Override","tree":{{"$className":"Folder","$path":"children"}}}}"#)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let response = client
                .get(format!("http://127.0.0.1:{port}/api/read/{root}"))
                .send()
                .unwrap();
            let body: serde_json::Value =
                rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
            let instances = body["instances"].as_object().unwrap();
            assert!(instances.values().any(|node| node["Name"] == "Override"));
            if instances.values().any(|node| node["Name"] == "NestedEmpty") == expected {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "removed nested override retained stale inheritance: {body}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
