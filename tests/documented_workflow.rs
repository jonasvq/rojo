#![cfg(feature = "cli")]

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use rbx_dom_weak::{
    types::{Enum, Variant, Vector3},
    WeakDom,
};

const FILES: &[&str] = &[
    "default.project.json",
    "src/shared/Greeting.luau",
    "src/shared/Greeting.meta.json",
    "src/server/Boot.server.luau",
    "src/client/Boot.client.luau",
    "assets/Demo.model.json",
];

fn run(folder: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(folder)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn check(dom: &WeakDom) {
    let model = dom
        .descendants()
        .find(|node| node.name == "Demo" && node.class == "Model")
        .unwrap();
    let pad = dom.get_by_ref(model.children()[0]).unwrap();
    assert_eq!(pad.name, "Pad");
    assert_eq!(
        pad.properties[&"Size".into()],
        Variant::Vector3(Vector3::new(12.0, 1.0, 12.0))
    );
    assert_eq!(
        model.properties[&"PrimaryPart".into()],
        Variant::Ref(pad.referent())
    );
    for class in ["StarterPlayerScripts", "StarterCharacterScripts"] {
        assert_eq!(
            dom.descendants().filter(|node| node.class == class).count(),
            1
        );
    }
    for context in [1, 2] {
        assert!(dom.descendants().any(|node| node.name == "Boot"
            && node.class == "Script"
            && node.properties.get(&"RunContext".into())
                == Some(&Variant::Enum(Enum::from_u32(context)))));
    }
    assert!(dom.descendants().any(|node| node.name=="Greeting" && node.class=="ModuleScript" && matches!(node.properties.get(&"Source".into()),Some(Variant::String(source)) if source.contains("Hello, %s!"))));
}

fn find<'a>(node: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    if node["name"] == name {
        return Some(node);
    }
    node.get("children")?
        .as_array()?
        .iter()
        .find_map(|child| find(child, name))
}

#[test]
fn tutorial_example_builds_maps_formats_and_previews_syncback_without_changing_sources() {
    let temporary = tempfile::tempdir().unwrap();
    let folder = temporary.path().join("project with spaces");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/agent-workflow");
    for file in FILES {
        let destination = folder.join(file);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(example.join(file), destination).unwrap();
    }
    for filename in ["output place.rbxlx", "output place.rbxl"] {
        let output = run(&folder, &["build", "default.project.json", "-o", filename]);
        assert!(!String::from_utf8_lossy(&output.stderr)
            .contains("StarterCharacterScripts without StarterPlayerScripts"));
        let bytes = fs::read(folder.join(filename)).unwrap();
        let dom = if filename.ends_with("rbxlx") {
            rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new()).unwrap()
        } else {
            rbx_binary::from_reader(bytes.as_slice()).unwrap()
        };
        check(&dom);
    }
    fs::create_dir_all(folder.join("build/out")).unwrap();
    run(
        &folder,
        &[
            "sourcemap",
            "default.project.json",
            "--include-non-scripts",
            "--relative-to-output",
            "-o",
            "build/out/sourcemap.json",
        ],
    );
    let map: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("build/out/sourcemap.json")).unwrap())
            .unwrap();
    let greeting = find(&map, "Greeting").unwrap();
    assert!(greeting["filePaths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| {
            let path = folder.join("build/out").join(path.as_str().unwrap());
            path.canonicalize().ok() == folder.join("src/shared/Greeting.luau").canonicalize().ok()
        }));
    let before: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("default.project.json")).unwrap()).unwrap();
    run(&folder, &["fmt-project", "default.project.json"]);
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("default.project.json")).unwrap()).unwrap();
    assert_eq!(
        before, after,
        "formatting changed example project semantics"
    );
    let saved: Vec<_> = FILES
        .iter()
        .map(|file| fs::read(folder.join(file)).unwrap())
        .collect();
    run(
        &folder,
        &[
            "syncback",
            "default.project.json",
            "--input",
            "output place.rbxlx",
            "--dry-run",
            "--list",
        ],
    );
    for (file, bytes) in FILES.iter().zip(saved) {
        assert_eq!(
            fs::read(folder.join(file)).unwrap(),
            bytes,
            "dry-run changed {file}"
        );
    }
}
