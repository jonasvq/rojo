use std::path::Path;

use librojo::{build_project, snapshot_from_vfs, InstanceContext, InstanceSnapshot, Project};
use memofs::{InMemoryFs, Vfs, VfsSnapshot};
use rbx_dom_weak::types::{Enum, Variant};
use serde_json::{json, Value};

const PROJECT: &str = "/project/default.project.json";

fn fixture() -> Vfs {
    let mut fs = InMemoryFs::new();
    fs.load_snapshot(
        "/project",
        VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file("{}")),
            (
                "src",
                VfsSnapshot::dir([
                    ("Client.client.luau", VfsSnapshot::file("return 1")),
                    ("Server.server.lua", VfsSnapshot::file("return 2")),
                    ("Module.lua", VfsSnapshot::file("return 3")),
                ]),
            ),
        ]),
    )
    .unwrap();
    Vfs::new(fs)
}

fn configuration(version: Option<u32>) -> Value {
    let mut value = json!({"name":"Root","tree":{"$path":"src"}});
    if let Some(version) = version {
        value["$version"] = version.into();
    }
    value
}

fn write(vfs: &Vfs, value: &Value) {
    vfs.write(PROJECT, serde_json::to_vec(value).unwrap())
        .unwrap();
}

fn snapshot(vfs: &Vfs, context: &InstanceContext) -> InstanceSnapshot {
    snapshot_from_vfs(context, vfs, Path::new(PROJECT))
        .unwrap()
        .unwrap()
}

fn child<'a>(snapshot: &'a InstanceSnapshot, name: &str) -> &'a InstanceSnapshot {
    snapshot
        .children
        .iter()
        .find(|node| node.name == name)
        .unwrap()
}

#[test]
fn explicit_versions_select_defaults_without_changing_unversioned_projects() {
    let vfs = fixture();
    for version in [None, Some(1), Some(2)] {
        write(&vfs, &configuration(version));
        let root = snapshot(&vfs, &InstanceContext::new());
        let modern = version == Some(2);
        assert_eq!(root.metadata.ignore_unknown_instances, modern);
        assert_eq!(
            child(&root, "Client").class_name,
            if modern { "Script" } else { "LocalScript" }
        );
        assert_eq!(
            child(&root, "Client").metadata.ignore_unknown_instances,
            modern
        );
        assert_eq!(child(&root, "Server").class_name, "Script");
        assert_eq!(child(&root, "Module").class_name, "ModuleScript");
        assert_eq!(
            child(&root, "Client").properties.get(&"RunContext".into()),
            modern.then_some(&Variant::Enum(Enum::from_u32(2)))
        );
        let project = Project::load_exact(&vfs, Path::new(PROJECT), None).unwrap();
        assert_eq!(
            serde_json::to_value(project).unwrap().get("$version"),
            version.map(Value::from).as_ref()
        );
    }
}

#[test]
fn invalid_versions_are_rejected_with_the_project_path() {
    let vfs = fixture();
    for version in [
        json!(0),
        json!(3),
        json!(-1),
        json!(2.5),
        json!(2.0),
        json!("2"),
        json!(null),
        json!(true),
        json!([]),
        json!({}),
    ] {
        let mut value = configuration(None);
        value["$version"] = version.clone();
        write(&vfs, &value);
        let error =
            anyhow::Error::new(Project::load_exact(&vfs, Path::new(PROJECT), None).unwrap_err());
        assert!(
            format!("{error:#}").contains(PROJECT),
            "version={version}: {error:#}"
        );
        assert!(
            format!("{error:#}").contains("$version"),
            "version={version}: {error:#}"
        );
    }
}

#[test]
fn version_defaults_do_not_persist_in_root_refresh_context() {
    let vfs = fixture();
    let mut context = InstanceContext::new();
    for version in [Some(2), Some(1), None, Some(2), None] {
        write(&vfs, &configuration(version));
        let root = snapshot(&vfs, &context);
        assert_eq!(root.metadata.ignore_unknown_instances, version == Some(2));
        assert_eq!(
            child(&root, "Client").class_name,
            if version == Some(2) {
                "Script"
            } else {
                "LocalScript"
            }
        );
        context = root.metadata.context;
        assert_eq!(context.ignore_unknown_descendants, None);
    }
}

#[test]
fn explicit_overrides_and_incoming_preservation_win_over_version_defaults() {
    let vfs = fixture();
    for version in [1, 2] {
        for legacy in [false, true] {
            let mut value = configuration(Some(version));
            value["emitLegacyScripts"] = legacy.into();
            value["tree"]["$ignoreUnknownDescendants"] = false.into();
            write(&vfs, &value);
            let root = snapshot(&vfs, &InstanceContext::new());
            assert!(!root.metadata.ignore_unknown_instances);
            assert!(!child(&root, "Client").metadata.ignore_unknown_instances);
            assert_eq!(
                child(&root, "Client").class_name,
                if legacy { "LocalScript" } else { "Script" }
            );
        }
        for policy in [false, true] {
            let mut context = InstanceContext::new();
            context.ignore_unknown_descendants = Some(policy);
            write(&vfs, &configuration(Some(version)));
            let root = snapshot(&vfs, &context);
            assert_eq!(root.metadata.ignore_unknown_instances, policy);
            assert_eq!(
                child(&root, "Client").metadata.ignore_unknown_instances,
                policy
            );
            assert_eq!(
                root.metadata.context.ignore_unknown_descendants,
                Some(policy)
            );
        }
    }
}

#[test]
fn loaded_project_versions_accept_bom_jsonc_and_drive_library_builds() {
    let vfs = fixture();
    vfs.write(
        PROJECT,
        b"\xef\xbb\xbf{\"$version\":2, /*opt in*/ \"name\":\"Root\",\"tree\":{\"$path\":\"src\"},}",
    )
    .unwrap();
    let project = Project::load_exact(&vfs, Path::new(PROJECT), None).unwrap();
    vfs.write(PROJECT, b"invalid physical project after loading")
        .unwrap();
    let tree = build_project(&vfs, &project).unwrap();
    let client = tree
        .descendants(tree.get_root_id())
        .find(|node| node.name() == "Client")
        .unwrap();
    assert_eq!(client.class_name(), "Script");
    assert!(client.metadata().ignore_unknown_instances);
}

#[test]
fn loaded_version_edits_and_native_metadata_rules_keep_their_precedence() {
    let vfs = fixture();
    write(&vfs, &configuration(None));
    let mut project = Project::load_exact(&vfs, Path::new(PROJECT), None).unwrap();
    project.format_version = Some(librojo::ProjectFormatVersion::V2);
    let tree = build_project(&vfs, &project).unwrap();
    assert!(tree
        .descendants(tree.get_root_id())
        .any(|node| node.name() == "Client" && node.class_name() == "Script"));
    vfs.write(
        "/project/src/Client.meta.json",
        br#"{"className":"LocalScript","ignoreUnknownInstances":false}"#,
    )
    .unwrap();
    vfs.write(
        "/project/src/Server.meta.jsonc",
        br#"{"properties":{"RunContext":"Legacy"},}"#,
    )
    .unwrap();
    project.sync_rules = serde_json::from_value(json!([
        {"pattern":"**/Module.lua","use":"legacyClientScript","suffix":".lua"}
    ]))
    .unwrap();
    let tree = build_project(&vfs, &project).unwrap();
    let client = tree
        .descendants(tree.get_root_id())
        .find(|node| node.name() == "Client")
        .unwrap();
    assert_eq!(client.class_name(), "LocalScript");
    assert!(!client.metadata().ignore_unknown_instances);
    let server = tree
        .descendants(tree.get_root_id())
        .find(|node| node.name() == "Server")
        .unwrap();
    assert_eq!(
        server.properties().get(&"RunContext".into()),
        Some(&Variant::Enum(Enum::from_u32(0)))
    );
    assert!(tree
        .descendants(tree.get_root_id())
        .any(|node| node.name() == "Module" && node.class_name() == "LocalScript"));
    assert_eq!(
        project.format_version,
        Some(librojo::ProjectFormatVersion::V2)
    );
}

#[test]
fn nested_versions_inherit_preservation_but_choose_their_own_emitter() {
    let vfs = fixture();
    for parent_version in [None, Some(1), Some(2)] {
        for nested_version in [None, Some(1), Some(2)] {
            for policy in [None, Some(false), Some(true)] {
                let mut root = configuration(parent_version);
                root["tree"] =
                    json!({"$className":"Folder","Nested":{"$path":"nested.project.json"}});
                if let Some(policy) = policy {
                    root["tree"]["$ignoreUnknownDescendants"] = policy.into();
                }
                write(&vfs, &root);
                vfs.write(
                    "/project/nested.project.json",
                    serde_json::to_vec(&configuration(nested_version)).unwrap(),
                )
                .unwrap();
                let root = snapshot(&vfs, &InstanceContext::new());
                let nested = &root.children[0];
                let client = child(nested, "Client");
                assert_eq!(
                    client.class_name,
                    if nested_version == Some(2) {
                        "Script"
                    } else {
                        "LocalScript"
                    }
                );
                let preserve =
                    policy.unwrap_or(parent_version == Some(2) || nested_version == Some(2));
                assert_eq!(
                    client.metadata.ignore_unknown_instances, preserve,
                    "parent={parent_version:?}, nested={nested_version:?}, policy={policy:?}"
                );
            }
        }
    }
}

#[test]
fn v2_syncback_twice_and_fmt_retain_version_and_native_context() {
    use librojo::{syncback_loop, RojoTree};
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    use std::{fs, process::Command};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    let project_path = dir.path().join("default.project.jsonc");
    fs::write(
        &project_path,
        br#"{"$version":2,"name":"Root","tree":{"$path":"src"},}"#,
    )
    .unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    for source in ["return 42", "return 43"] {
        let project = Project::load_exact(&vfs, &project_path, None).unwrap();
        let before = snapshot_from_vfs(&InstanceContext::new(), &vfs, &project_path)
            .unwrap()
            .unwrap();
        let mut old = RojoTree::new(before);
        let new = WeakDom::new(
            InstanceBuilder::new("Folder").with_name("Root").with_child(
                InstanceBuilder::new("Script")
                    .with_name("Added")
                    .with_property("Source", source)
                    .with_property("RunContext", Enum::from_u32(2)),
            ),
        );
        let changes = syncback_loop(&vfs, &mut old, new, &project).unwrap();
        changes.write_to_vfs(dir.path(), &vfs).unwrap();
        let after = snapshot_from_vfs(&InstanceContext::new(), &vfs, &project_path)
            .unwrap()
            .unwrap();
        assert_eq!(child(&after, "Added").class_name, "Script");
        assert_eq!(
            child(&after, "Added").properties.get(&"RunContext".into()),
            Some(&Variant::Enum(Enum::from_u32(2)))
        );
        assert_eq!(
            child(&after, "Added").properties.get(&"Source".into()),
            Some(&Variant::String(source.into()))
        );
        assert_eq!(
            Project::load_exact(&vfs, &project_path, None)
                .unwrap()
                .format_version,
            Some(librojo::ProjectFormatVersion::V2)
        );
    }
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .args(["fmt-project"])
        .arg(&project_path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let formatted: Value = serde_json::from_slice(&fs::read(project_path).unwrap()).unwrap();
    assert_eq!(formatted["$version"], 2);
    assert!(formatted.get("emitLegacyScripts").is_none());
    assert!(formatted["tree"].get("$ignoreUnknownDescendants").is_none());
}

#[test]
fn live_version_changes_retain_identity_and_do_not_leave_stale_defaults() {
    use std::{
        fs,
        net::TcpListener,
        process::{Command, Stdio},
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
    fs::write(dir.path().join("src/Client.client.luau"), "return 1").unwrap();
    let project = dir.path().join("default.project.json");
    fs::write(
        &project,
        serde_json::to_vec(&configuration(Some(2))).unwrap(),
    )
    .unwrap();
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
            .stderr(fs::File::create(dir.path().join("server.log")).unwrap())
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
                let body: Value = rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
                break body["rootInstanceId"].as_str().unwrap().to_owned();
            }
        }
        assert!(Instant::now() < deadline && server.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut source_id = None;
    for version in [Some(2), Some(1), None, Some(2), None] {
        fs::write(
            &project,
            serde_json::to_vec(&configuration(version)).unwrap(),
        )
        .unwrap();
        let modern = version == Some(2);
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let response = client
                .get(format!("http://127.0.0.1:{port}/api/read/{root}"))
                .send()
                .unwrap();
            assert!(response.status().is_success());
            let body: Value = rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
            let instances = body["instances"].as_object().unwrap();
            let (id, source) = instances
                .iter()
                .find(|(_, node)| node["Name"] == "Client")
                .unwrap();
            if source["ClassName"] == if modern { "Script" } else { "LocalScript" }
                && source["Metadata"]["ignoreUnknownInstances"] == modern
                && instances[&root]["Metadata"]["ignoreUnknownInstances"] == modern
            {
                if let Some(previous) = &source_id {
                    assert_eq!(previous, id);
                }
                source_id = Some(id.clone());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "version={version:?}, read={body}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fs::write(dir.path().join("src/Canary.lua"), "return 99").unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let response = client
            .get(format!("http://127.0.0.1:{port}/api/read/{root}"))
            .send()
            .unwrap();
        let body: Value = rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
        if body["instances"]
            .as_object()
            .unwrap()
            .values()
            .any(|node| node["Name"] == "Canary")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "canary missing after version reloads: {body}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn versioned_place_overlays_preserve_unknowns_only_under_the_selected_policy() {
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    use std::{fs, process::Command};
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/Known.lua"), "return 1").unwrap();
    let input = dir.path().join("input.rbxlx");
    let dom = WeakDom::new(
        InstanceBuilder::new("DataModel").with_child(
            InstanceBuilder::new("ServerStorage")
                .with_child(InstanceBuilder::new("Folder").with_name("StudioOnly")),
        ),
    );
    rbx_xml::to_writer(
        fs::File::create(&input).unwrap(),
        &dom,
        dom.root().children(),
        rbx_xml::EncodeOptions::new(),
    )
    .unwrap();
    for (version, policy, preserve) in [
        (None, None, false),
        (Some(1), None, false),
        (Some(2), None, true),
        (Some(2), Some(false), false),
        (Some(1), Some(true), true),
    ] {
        let mut config = configuration(version);
        config["tree"] = json!({"$className":"DataModel","ServerStorage":{"$path":"src"}});
        if let Some(policy) = policy {
            config["tree"]["ServerStorage"]["$ignoreUnknownDescendants"] = policy.into();
        }
        fs::write(
            dir.path().join("default.project.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", "--input", "input.rbxlx", "-o", "output.rbxlx"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = rbx_xml::from_reader(
            fs::File::open(dir.path().join("output.rbxlx")).unwrap(),
            rbx_xml::DecodeOptions::new(),
        )
        .unwrap();
        assert_eq!(
            result.descendants().any(|node| node.name == "StudioOnly"),
            preserve,
            "version={version:?}, policy={policy:?}"
        );
        assert!(result.descendants().any(|node| node.name == "Known"));
    }
}
