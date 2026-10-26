use std::{fs, path::Path, process::Command};

use insta::assert_snapshot;
use tempfile::tempdir;

use crate::rojo_test::io_util::{get_working_dir_path, BUILD_TESTS_PATH, ROJO_PATH};

#[test]
fn self_referential_project_returns_a_contextual_cycle_error() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("default.project.json");
    fs::write(&project, r#"{"name":"Cycle","tree":{"$path":"."}}"#).unwrap();
    let output = Command::new(ROJO_PATH)
        .arg("build")
        .arg(&project)
        .arg("--output")
        .arg(dir.path().join("output.rbxmx"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("Circular project reference"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("default.project.json"), "{diagnostic}");
    assert!(!diagnostic.contains("overflowed its stack"), "{diagnostic}");
}

macro_rules! gen_build_tests {
    ( $($test_name: ident,)* ) => {
        $(
            paste::item! {
                #[test]
                fn [<build_ $test_name>]() {
                    let _ = env_logger::try_init();

                    run_build_test(stringify!($test_name));
                }
            }
        )*
    };
}

gen_build_tests! {
    init_csv_with_children,
    attributes,
    client_in_folder,
    client_init,
    csv_bug_145,
    csv_bug_147,
    csv_in_folder,
    deep_nesting,
    gitkeep,
    ignore_glob_inner,
    ignore_glob_nested,
    ignore_glob_spec,
    infer_service_name,
    infer_starter_player,
    init_meta_class_name,
    init_meta_properties,
    init_with_children,
    issue_546,
    json_as_lua,
    json_model_in_folder,
    json_model_legacy_name,
    module_in_folder,
    module_init,
    nested_runcontext,
    optional,
    project_composed_default,
    project_composed_file,
    project_root_name,
    rbxm_in_folder,
    rbxmx_in_folder,
    rbxmx_ref,
    script_meta_disabled,
    server_in_folder,
    server_init,
    txt,
    txt_in_folder,
    unresolved_values,
    weldconstraint,
    sync_rule_alone,
    sync_rule_complex,
    sync_rule_nested_projects,
    no_name_default_project,
    no_name_project,
    no_name_top_level_project,
    plugin_init,
}

#[cfg(windows)]
#[test]
fn build_rejects_invalid_output_names_before_creating_files() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"Special: Character","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    for name in [
        "Special: Character.rbxm",
        "CON.rbxm",
        "con.rbxm",
        "Lpt1.rbxm",
        "bad?.rbxm",
    ] {
        let output = Command::new(ROJO_PATH)
            .arg("build")
            .arg(&project)
            .arg("--output")
            .arg(dir.path().join(name))
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "invalid output name {name} was reported successful"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Invalid output file name") && error.contains(name),
            "{error}"
        );
        assert!(
            !dir.path().join("Special").exists(),
            "an alternate data stream created a misleading base file"
        );
    }
    let output = Command::new(ROJO_PATH)
        .arg("build")
        .arg(&project)
        .arg("--output")
        .arg(dir.path().join("valid.rbxm"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "valid Roblox instance names must remain accepted"
    );
}

#[test]
fn build_warns_for_duplicate_path_bindings_without_removing_aliases() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("shared.luau"), "return 1").unwrap();
    fs::write(dir.path().join("other.luau"), "return 2").unwrap();
    let absolute = dir.path().join("shared.luau");
    for (second, duplicate) in [
        ("./shared.luau", true),
        (absolute.to_str().unwrap(), true),
        ("other.luau", false),
    ] {
        let project = dir.path().join("default.project.json");
        fs::write(
            &project,
            serde_json::to_vec(&serde_json::json!({
                "name": "Aliases", "tree": {"$className": "Folder",
                    "A": {"$path": "shared.luau"}, "B": {"$path": second}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let output_path = dir.path().join("out.rbxm");
        let output = Command::new(ROJO_PATH)
            .arg("build")
            .arg(&project)
            .arg("-o")
            .arg(&output_path)
            .env("RUST_LOG", "warn")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr.contains("Duplicate $path binding"),
            duplicate,
            "{stderr}"
        );
        if duplicate {
            for detail in ["shared.luau", "default.project.json", "tree/A", "tree/B"] {
                assert!(stderr.contains(detail), "Missing {detail}: {stderr}");
            }
        }
        let model = rbx_binary::from_reader(fs::File::open(&output_path).unwrap()).unwrap();
        let root = model.get_by_ref(model.root().children()[0]).unwrap();
        let names: Vec<_> = root
            .children()
            .iter()
            .map(|id| model.get_by_ref(*id).unwrap().name.as_str())
            .collect();
        assert_eq!(names, ["A", "B"]);
    }
}

#[test]
fn binary_build_preserves_legacy_physics_volume() {
    use rbx_dom_weak::{types::Variant, ustr, InstanceBuilder, WeakDom};

    let directory = tempdir().unwrap();
    for (name, migrated) in [
        ("Legacy", None),
        ("Modern", Some(true)),
        ("Unmigrated", Some(false)),
    ] {
        let mut builder = InstanceBuilder::new("MeshPart").with_name(name);
        if let Some(value) = migrated {
            builder = builder
                .with_property("InertiaMigrated", value)
                .with_property("UnscaledVolume", if value { 4.0_f32 } else { 7.0_f32 });
        }
        let model = WeakDom::new(builder);
        let mut bytes = Vec::new();
        rbx_binary::to_writer(&mut bytes, &model, &[model.root_ref()]).unwrap();
        fs::write(directory.path().join(format!("{name}.rbxm")), bytes).unwrap();
    }
    let project = directory.path().join("default.project.json");
    fs::write(&project, r#"{"name":"Root","tree":{"$className":"Folder","Legacy":{"$path":"Legacy.rbxm"},"Modern":{"$path":"Modern.rbxm"},"Unmigrated":{"$path":"Unmigrated.rbxm"}}}"#).unwrap();
    for extension in ["rbxm", "rbxl"] {
        let output_path = directory.path().join(format!("built.{extension}"));
        let output = Command::new(ROJO_PATH)
            .arg("build")
            .arg(&project)
            .arg("--output")
            .arg(&output_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = fs::read(output_path).unwrap();
        let model = rbx_binary::from_reader(bytes.as_slice()).unwrap();
        for (name, expected) in [("Legacy", 0.0_f32), ("Modern", 4.0), ("Unmigrated", 7.0)] {
            let part = model
                .descendants()
                .find(|instance| instance.name == name)
                .unwrap();
            assert_eq!(
                part.properties.get(&ustr("UnscaledVolume")),
                Some(&Variant::Float32(expected)),
                "{extension}: {name}"
            );
            if name != "Legacy" {
                assert_eq!(
                    part.properties.get(&ustr("InertiaMigrated")),
                    Some(&Variant::Bool(name == "Modern"))
                );
            }
        }
    }
}

fn run_build_test(test_name: &str) {
    let working_dir = get_working_dir_path();

    let input_path = Path::new(BUILD_TESTS_PATH).join(test_name);

    let output_dir = tempdir().expect("couldn't create temporary directory");
    let output_path = output_dir.path().join(format!("{}.rbxmx", test_name));

    let output = Command::new(ROJO_PATH)
        .args([
            "build",
            input_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .env("RUST_LOG", "error")
        .current_dir(working_dir)
        .output()
        .expect("Couldn't start Rojo");

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));

    assert!(output.status.success(), "Rojo did not exit successfully");

    let contents = fs::read_to_string(&output_path).expect("Couldn't read output file");

    let mut settings = insta::Settings::new();

    let snapshot_path = Path::new(BUILD_TESTS_PATH)
        .parent()
        .unwrap()
        .join("build-test-snapshots");

    settings.set_snapshot_path(snapshot_path);

    settings.bind(|| {
        assert_snapshot!(test_name, contents);
    });
}
