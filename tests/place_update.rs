use rbx_dom_weak::{InstanceBuilder, WeakDom};
use std::{fs, path::Path, process::Command};

fn write_place(path: &Path, dom: &WeakDom) {
    let file = fs::File::create(path).unwrap();
    rbx_xml::to_writer(
        file,
        dom,
        dom.root().children(),
        rbx_xml::EncodeOptions::new(),
    )
    .unwrap();
}

#[test]
fn existing_place_build_preserves_unmapped_services_and_unspecified_properties() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.rbxlx");
    let output = dir.path().join("output.rbxlx");
    let project = dir.path().join("default.project.json");
    let dom = WeakDom::new(
        InstanceBuilder::new("DataModel")
            .with_child(
                InstanceBuilder::new("Workspace")
                    .with_child(InstanceBuilder::new("Part").with_name("Preserved")),
            )
            .with_child(
                InstanceBuilder::new("ReplicatedStorage")
                    .with_child(
                        InstanceBuilder::new("Folder")
                            .with_name("Managed")
                            .with_property("Archivable", false)
                            .with_child(
                                InstanceBuilder::new("StringValue")
                                    .with_name("Existing")
                                    .with_property("Value", "old"),
                            ),
                    )
                    .with_child(InstanceBuilder::new("Folder").with_name("Other")),
            ),
    );
    write_place(&input, &dom);
    fs::write(&project,r#"{"name":"Root","tree":{"$className":"DataModel","ReplicatedStorage":{"$className":"ReplicatedStorage","Managed":{"$className":"Folder","Existing":{"$className":"StringValue","$properties":{"Value":"updated"}}}}}}"#).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("build")
        .arg(&project)
        .arg("--input")
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let updated = rbx_xml::from_reader(
        fs::File::open(output).unwrap(),
        rbx_xml::DecodeOptions::new(),
    )
    .unwrap();
    let values: Vec<_> = updated.descendants().collect();
    assert!(values.iter().any(|instance| instance.name == "Preserved"));
    assert!(values.iter().any(|instance| instance.name == "Other"));
    let managed = values
        .iter()
        .find(|instance| instance.name == "Managed")
        .unwrap();
    assert_eq!(
        managed.properties.get(&"Archivable".into()),
        Some(&rbx_dom_weak::types::Variant::Bool(false))
    );
    let existing = values
        .iter()
        .find(|instance| instance.name == "Existing")
        .unwrap();
    assert_eq!(
        existing.properties.get(&"Value".into()),
        Some(&rbx_dom_weak::types::Variant::String("updated".into()))
    );
}

use rbx_dom_weak::types::{Ref, Variant};

fn update(project: &Path, input: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("build")
        .arg(project)
        .arg("--input")
        .arg(input)
        .arg("--output")
        .arg(output)
        .output()
        .unwrap()
}

fn read_place(path: &Path) -> WeakDom {
    let bytes = fs::read(path).unwrap();
    if path.extension().unwrap() == "rbxl" {
        rbx_binary::from_reader(bytes.as_slice()).unwrap()
    } else {
        rbx_xml::from_reader(
            bytes.as_slice(),
            rbx_xml::DecodeOptions::new()
                .property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown),
        )
        .unwrap()
    }
}

fn named<'a>(dom: &'a WeakDom, name: &str) -> &'a rbx_dom_weak::Instance {
    dom.descendants()
        .find(|instance| instance.name == name)
        .unwrap()
}

#[test]
fn place_overlay_respects_deletion_overrides_references_attributes_and_order() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.rbxlx");
    let output = dir.path().join("output.rbxlx");
    let project = dir.path().join("default.project.json");
    let keep = Ref::new();
    let remove = Ref::new();
    let mut dom = WeakDom::new(
        InstanceBuilder::new("DataModel")
            .with_child(
                InstanceBuilder::new("Workspace")
                    .with_child(
                        InstanceBuilder::new("ObjectValue")
                            .with_name("SurvivingLink")
                            .with_property("Value", keep),
                    )
                    .with_child(
                        InstanceBuilder::new("ObjectValue")
                            .with_name("DeletedLink")
                            .with_property("Value", remove),
                    ),
            )
            .with_child(
                InstanceBuilder::new("ReplicatedStorage")
                    .with_child(
                        InstanceBuilder::new("Folder")
                            .with_name("Managed")
                            .with_child(
                                InstanceBuilder::new("Folder")
                                    .with_name("Keep")
                                    .with_referent(keep)
                                    .with_child(
                                        InstanceBuilder::new("Folder")
                                            .with_name("UnmanagedGrandchild"),
                                    ),
                            )
                            .with_child(
                                InstanceBuilder::new("Folder")
                                    .with_name("Remove")
                                    .with_referent(remove),
                            )
                            .with_child(
                                InstanceBuilder::new("ObjectValue")
                                    .with_name("Outbound")
                                    .with_property("Value", remove),
                            ),
                    )
                    .with_child(InstanceBuilder::new("Folder").with_name("UnmanagedSibling")),
            ),
    );
    let managed_id = named(&dom, "Managed").referent();
    dom.insert(
        managed_id,
        InstanceBuilder::new("ObjectValue")
            .with_name("Nil")
            .with_property("Value", keep),
    );
    let mut attrs = rbx_dom_weak::types::Attributes::new();
    attrs.insert("OldAttribute".into(), true.into());
    dom.get_by_ref_mut(keep)
        .unwrap()
        .properties
        .insert("Attributes".into(), attrs.into());
    write_place(&input, &dom);
    fs::write(&project,serde_json::to_vec(&serde_json::json!({"name":"Root","tree":{
        "$className":"DataModel","ReplicatedStorage":{"$className":"ReplicatedStorage","Managed":{
            "$className":"Folder","$ignoreUnknownInstances":false,
            "Keep":{"$className":"Folder","$ignoreUnknownInstances":true,"$attributes":{"Rojo_Id":"keep-target","NewAttribute":true},"$properties":{"Tags":["new-tag"]}},
            "Created":{"$className":"Folder","$attributes":{"Rojo_Id":"new-target"}},
            "Owner":{"$className":"ObjectValue","$attributes":{"Rojo_Target_Value":"new-target"}},
            "Outbound":{"$className":"ObjectValue","$attributes":{"Rojo_Target_Value":"keep-target"}},
            "Nil":{"$className":"ObjectValue","$attributes":{"Rojo_Target_Value":"missing-target"}}
        }}}})).unwrap()).unwrap();
    let result = update(&project, &input, &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("ReplicatedStorage.Managed.Remove"));
    let dom = read_place(&output);
    assert!(!dom.descendants().any(|instance| instance.name == "Remove"));
    let keep = named(&dom, "Keep");
    let created = named(&dom, "Created");
    assert!(dom
        .descendants()
        .any(|instance| instance.name == "UnmanagedGrandchild"));
    assert!(dom
        .descendants()
        .any(|instance| instance.name == "UnmanagedSibling"));
    for (owner, target) in [
        ("SurvivingLink", keep.referent()),
        ("DeletedLink", Ref::none()),
        ("Owner", created.referent()),
        ("Outbound", keep.referent()),
        ("Nil", Ref::none()),
    ] {
        assert_eq!(
            named(&dom, owner).properties.get(&"Value".into()),
            Some(&Variant::Ref(target)),
            "{owner}"
        );
    }
    let Some(Variant::Attributes(attrs)) = keep.properties.get(&"Attributes".into()) else {
        panic!("missing attrs")
    };
    assert!(attrs.get("OldAttribute").is_none());
    assert!(attrs.get("NewAttribute").is_some());
    assert!(
        matches!(keep.properties.get(&"Tags".into()),Some(Variant::Tags(tags)) if tags.iter().any(|tag|tag=="new-tag"))
    );
    let children: Vec<_> = named(&dom, "Managed")
        .children()
        .iter()
        .map(|id| dom.get_by_ref(*id).unwrap().name.as_str())
        .collect();
    assert_eq!(&children[..3], &["Keep", "Outbound", "Nil"]);
    assert_eq!(children.len(), 5);
    let binary = dir.path().join("binary.rbxl");
    let result = update(&project, &output, &binary);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let binary = read_place(&binary);
    for (owner, target) in [
        ("Owner", named(&binary, "Created").referent()),
        ("Outbound", named(&binary, "Keep").referent()),
        ("SurvivingLink", named(&binary, "Keep").referent()),
        ("DeletedLink", Ref::none()),
        ("Nil", Ref::none()),
    ] {
        assert_eq!(
            named(&binary, owner).properties.get(&"Value".into()),
            Some(&Variant::Ref(target)),
            "binary {owner}"
        );
    }
}

#[test]
fn place_overlay_converts_formats_and_repeats_same_path_updates() {
    for input_extension in ["rbxl", "rbxlx"] {
        for output_extension in ["rbxl", "rbxlx"] {
            let dir = tempfile::tempdir().unwrap();
            let project = dir.path().join("default.project.json");
            let input = dir.path().join(format!("input.{input_extension}"));
            let output = dir.path().join(format!("output.{output_extension}"));
            let dom = WeakDom::new(
                InstanceBuilder::new("DataModel").with_child(
                    InstanceBuilder::new("Workspace")
                        .with_child(InstanceBuilder::new("Part").with_name("Preserved")),
                ),
            );
            if input_extension == "rbxl" {
                rbx_binary::to_writer(
                    fs::File::create(&input).unwrap(),
                    &dom,
                    dom.root().children(),
                )
                .unwrap();
            } else {
                write_place(&input, &dom);
            }
            for stage in [1, 2] {
                fs::write(&project,serde_json::to_vec(&serde_json::json!({"name":"Root","tree":{"$className":"DataModel",
                    "ReplicatedStorage":{"$className":"ReplicatedStorage","Value":{"$className":"IntValue","$properties":{"Value":stage}}}}})).unwrap()).unwrap();
                let from = if stage == 1 { &input } else { &output };
                let result = update(&project, from, &output);
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let dom = read_place(&output);
                assert!(dom
                    .descendants()
                    .any(|instance| instance.name == "Preserved"));
                assert_eq!(
                    named(&dom, "Value").properties.get(&"Value".into()),
                    Some(&Variant::Int64(stage))
                );
            }
            assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".rojo-")));
        }
    }
}

#[test]
fn place_overlay_rejects_ambiguity_class_mismatch_and_malformed_inputs_without_output_changes() {
    for case in ["duplicate", "class", "malformed", "root", "model"] {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("default.project.json");
        let input = dir.path().join(if case == "model" {
            "input.rbxm"
        } else {
            "input.rbxlx"
        });
        let output = dir.path().join("output.rbxlx");
        let mut storage = InstanceBuilder::new("ReplicatedStorage").with_child(
            InstanceBuilder::new(if case == "class" { "Part" } else { "Folder" })
                .with_name("Managed"),
        );
        if case == "duplicate" {
            storage = storage.with_child(InstanceBuilder::new("Folder").with_name("Managed"));
        }
        let dom = WeakDom::new(InstanceBuilder::new("DataModel").with_child(storage));
        write_place(&input, &dom);
        if case == "malformed" {
            fs::write(&input, b"incomplete").unwrap();
        }
        let before = fs::read(&input).unwrap();
        fs::write(&output, b"preserve original output").unwrap();
        fs::write(&project,serde_json::to_vec(&serde_json::json!({"name":"Root","tree":{"$className":if case=="root" {"Folder"} else {"DataModel"},
            "ReplicatedStorage":{"$className":"ReplicatedStorage","Managed":{"$className":"Folder"}}}})).unwrap()).unwrap();
        let result = update(&project, &input, &output);
        assert!(!result.status.success(), "{case}");
        assert_eq!(
            fs::read(&output).unwrap(),
            b"preserve original output",
            "{case}"
        );
        assert_eq!(fs::read(input).unwrap(), before, "{case}");
    }
}

#[test]
fn place_overlay_cli_requires_output_and_rejects_watch_and_plugin() {
    use clap::Parser;
    use librojo::cli::Options;
    assert!(Options::try_parse_from([
        "rojo",
        "build",
        "--input",
        "input.rbxl",
        "--output",
        "output.rbxl"
    ])
    .is_ok());
    for args in [
        vec!["rojo", "build", "--input", "input.rbxl"],
        vec![
            "rojo",
            "build",
            "--input",
            "input.rbxl",
            "--output",
            "output.rbxl",
            "--watch",
        ],
        vec![
            "rojo",
            "build",
            "--input",
            "input.rbxl",
            "--plugin",
            "test.rbxm",
        ],
    ] {
        assert!(Options::try_parse_from(args).is_err());
    }
}

#[test]
fn existing_model_migration_state_is_not_reset_by_a_generated_build_default() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.rbxlx");
    let output = dir.path().join("output.rbxlx");
    let project = dir.path().join("default.project.json");
    let dom = WeakDom::new(
        InstanceBuilder::new("DataModel").with_child(
            InstanceBuilder::new("Workspace").with_child(
                InstanceBuilder::new("Model")
                    .with_name("ManagedModel")
                    .with_property("NeedsPivotMigration", true),
            ),
        ),
    );
    write_place(&input, &dom);
    fs::write(&project,r#"{"name":"Root","tree":{"$className":"DataModel","Workspace":{"$className":"Workspace","ManagedModel":{"$className":"Model"}}}}"#).unwrap();
    let result = update(&project, &input, &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let dom = read_place(&output);
    assert_eq!(
        named(&dom, "ManagedModel")
            .properties
            .get(&"NeedsPivotMigration".into()),
        Some(&Variant::Bool(true))
    );
    let text = fs::read_to_string(&project).unwrap().replace(
        "\"$className\":\"Model\"",
        "\"$className\":\"Model\",\"$properties\":{\"NeedsPivotMigration\":false}",
    );
    fs::write(&project, text).unwrap();
    let result = update(&project, &input, &output);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let dom = read_place(&output);
    assert_eq!(
        named(&dom, "ManagedModel")
            .properties
            .get(&"NeedsPivotMigration".into()),
        Some(&Variant::Bool(false))
    );
}
