#![cfg(feature = "cli")]

use std::{fs, process::Command};

use serde_json::json;

#[test]
fn place_build_warns_for_missing_player_scripts_without_inserting_containers() {
    let dir = tempfile::tempdir().unwrap();
    let project = json!({"name":"Root","tree":{"$className":"DataModel","StarterPlayer":{"$className":"StarterPlayer","StarterCharacterScripts":{"$className":"StarterCharacterScripts"}}}});
    fs::write(dir.path().join("default.project.json"), project.to_string()).unwrap();
    for output_name in ["out.rbxlx", "out.rbxl"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", "-o", output_name])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("StarterCharacterScripts without StarterPlayerScripts"),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("Declare both containers"),
            "{diagnostic}"
        );
        let bytes = fs::read(dir.path().join(output_name)).unwrap();
        let dom = if output_name.ends_with("rbxlx") {
            rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new()).unwrap()
        } else {
            rbx_binary::from_reader(bytes.as_slice()).unwrap()
        };
        assert_eq!(
            dom.descendants()
                .filter(|node| node.class == "StarterPlayerScripts")
                .count(),
            0
        );
        assert_eq!(
            dom.descendants()
                .filter(|node| node.class == "StarterCharacterScripts")
                .count(),
            1
        );
    }
}

#[test]
fn final_source_tree_suppresses_warning_when_player_scripts_comes_from_a_model() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("player.model.json"),r#"{"className":"StarterPlayer","children":[{"name":"StarterCharacterScripts","className":"StarterCharacterScripts"},{"name":"StarterPlayerScripts","className":"StarterPlayerScripts"}]}"#).unwrap();
    fs::write(dir.path().join("default.project.json"),json!({"name":"Root","tree":{"$className":"DataModel","StarterPlayer":{"$path":"player.model.json"}}}).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "-o", "out.rbxlx"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr)
        .contains("StarterCharacterScripts without StarterPlayerScripts"));
}
