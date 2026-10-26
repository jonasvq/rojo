use std::ffi::OsStr;

use insta::assert_snapshot;

use crate::rojo_test::syncback_util::{run_syncback_test, snapshot_rbxm};

macro_rules! syncback_tests {
    ($($test_name:ident => $list:expr$(,)?),*) => {$(
        #[test]
        fn $test_name() {
            run_syncback_test(stringify!($test_name), |path| {
                for name in $list {
                    let snapshot_name = format!(concat!(stringify!($test_name), "-{}"), name);
                    let new = path.join::<&str>(name);
                    if !new.exists() {
                        panic!("the path stub '{}' does not exist after syncback runs. consider double checking for typos.", name);
                    }
                    if let Some("rbxm") = new.extension().and_then(OsStr::to_str) {
                        let content = fs_err::read(new).unwrap();
                        snapshot_rbxm(&snapshot_name, content, name);
                    } else {
                        let content = fs_err::read_to_string(new).unwrap();
                        assert_snapshot!(snapshot_name, content, name);
                    }
                }
            });
        }
    )*};
}

syncback_tests! {
    // Ensures that there's only one copy written to disk if navigating a
    // project file might yield two copies
    child_but_not => ["OnlyOneCopy/child_of_one.luau", "ReplicatedStorage/child_replicated_storage.luau"],
    // Ensures that syncback works with CSVs
    csv => ["src/csv_init/init.csv", "src/csv.csv"],
    // Ensures that if a RojoId is duplicated somewhere in the project, it's
    // rewritten rather than synced back as a conflict
    duplicate_rojo_id => ["container.model.json"],
    // Ensures that the `ignorePaths` setting works for additions
    ignore_paths_adding => ["src/int_value.model.json", "src/subfolder/string_value.txt"],
    // Ensures that the `ignorePaths` setting works for `init` files
    ignore_paths_init => ["src/non-init.luau", "src/init-file/init.luau"],
    // Ensures that the `ignorePaths` setting works for removals
    ignore_paths_removing => ["src/Message.rbxm"],
    // Ensures that `ignoreTrees` works for additions
    ignore_trees_adding => [],
    // Ensures that `ignoreTrees` works for removals
    ignore_trees_removing => [],
    // Ensures that all of the JSON middlewares are handled as expected
    json_middlewares => ["src/dir_with_meta/init.meta.json", "src/model_json.model.json", "src/project_json.project.json"],
    // Ensures projects that refer to other projects work as expected
    nested_projects => ["nested.project.json", "string_value.txt"],
    // Ensures files that are ignored by nested projects are picked up if
    // they're included in second project. Unusual but perfectly workable
    // pattern that syncback has to support.
    nested_projects_weird => ["src/modules/ClientModule.luau", "src/modules/ServerModule.luau"],
    // Ensures that projects respect `init` files when they're directly referenced from a node
    project_init => ["src/init.luau"],
    // Ensures that projects can be reserialized by syncback and that
    // default.project.json doesn't change unexpectedly.
    project_reserialize => ["attribute_mismatch.luau", "property_mismatch.project.json"],
    // Confirms that Instances that cannot serialize as directories serialize as rbxms
    rbxm_fallback => ["src/ChildWithDuplicates.rbxm"],
    // Ensures that ref properties are linked properly on the file system
    ref_properties => ["src/pointer.model.json", "src/target.model.json"],
    // Ensures that ref properties are linked when no attributes are manually
    // set in the DataModel
    ref_properties_blank => ["src/pointer.model.json", "src/target.meta.json", "src/target.txt"],
    // Ensures that if there is a conflict in RojoRefs, one of them is rewritten.
    ref_properties_conflict => ["src/Pointer_2.model.json", "src/Target_2.model.json"],
    // Ensures that having multiple pointers that are aimed at the same target doesn't trigger ref rewrites.
    ref_properties_duplicate => [],
    // Ensures that ref properties that point to nothing after the prune both
    // do not leave any trace of themselves
    ref_properties_pruned => ["src/Pointer1.model.json", "src/Pointer2.model.json", "src/Pointer3.model.json"],
    // Ensures that the old middleware is respected during syncback
    respect_old_middleware => ["default.project.json", "src/model_json.model.json", "src/rbxm.rbxm", "src/rbxmx.rbxmx"],
    // Ensures that the `$schema` field roundtrips with syncback
    schema_roundtrip => ["default.project.json", "src/model.model.json", "src/init/init.meta.json", "src/adjacent.meta.json"],
    // Ensures that StringValues inside project files are written to the
    // project file, but only if they don't have `$path` set
    string_value_project => ["default.project.json"],
    // Ensures that sync rules are respected. This is really just a test to
    // ensure it uses the old path when possible, but we want the coverage.
    sync_rules => ["src/module.modulescript", "src/text.text"],
    // Ensures that the `syncUnscriptable` setting works
    unscriptable_properties => ["default.project.json"],
    // Ensures that syncback correctly removes default values from projects rather
    // than leaving an incorrect value.
    project_default_properties_remove => ["default.project.json"],
}

#[test]
fn syncback_json_properties_are_stable_across_processes() {
    use crate::rojo_test::io_util::ROJO_PATH;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    use std::{fs, process::Command};

    let dir = tempfile::tempdir().unwrap();
    let mut attributes = rbx_dom_weak::types::Attributes::new();
    attributes.insert("Zeta".into(), 3.0f64.into());
    attributes.insert("Alpha".into(), "kept".into());
    let tool = |name: &str| {
        InstanceBuilder::new("Tool")
            .with_name(name)
            .with_property("CanBeDropped", false)
            .with_property("RequiresHandle", false)
            .with_property("Enabled", false)
            .with_property("ManualActivationOnly", true)
            .with_property("ToolTip", "Example")
            .with_property("Attributes", attributes.clone())
    };
    let model = WeakDom::new(
        InstanceBuilder::new("Folder")
            .with_name("Root")
            .with_child(
                tool("Directory").with_child(InstanceBuilder::new("Folder").with_name("Child")),
            )
            .with_child(tool("Model").with_child(tool("Nested")))
            .with_child(
                InstanceBuilder::new("Script")
                    .with_name("Script")
                    .with_property("Source", "return 1")
                    .with_property("Disabled", true)
                    .with_property("LinkedSource", "rbxassetid://123"),
            ),
    );
    let input = dir.path().join("input.rbxm");
    rbx_binary::to_writer(
        fs::File::create(&input).unwrap(),
        &model,
        &[model.root_ref()],
    )
    .unwrap();
    let project = dir.path().join("default.project.json");
    fs::write(&project, r#"{"name":"Root","tree":{"$path":"src"}}"#).unwrap();
    let mut baseline = None;
    for _ in 0..8 {
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("Directory")).unwrap();
        fs::write(
            src.join("Directory/init.meta.json"),
            r#"{"className":"Tool"}"#,
        )
        .unwrap();
        fs::write(
            src.join("Directory/Child.model.json"),
            r#"{"className":"Folder"}"#,
        )
        .unwrap();
        fs::write(src.join("Model.model.json"), r#"{"className":"Tool"}"#).unwrap();
        // Keep the serialization fixture within a native Script binding;
        // ModuleScript-to-Script layout conversions are rejected separately.
        fs::write(src.join("Script.server.luau"), "return 0").unwrap();
        let output = Command::new(ROJO_PATH)
            .arg("syncback")
            .arg(&project)
            .arg("--input")
            .arg(&input)
            .arg("--non-interactive")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let files: Vec<_> = [
            "Directory/init.meta.json",
            "Model.model.json",
            "Script.meta.json",
        ]
        .iter()
        .map(|name| fs::read_to_string(src.join(name)).unwrap())
        .collect();
        for contents in &files[..2] {
            let json: serde_json::Value = serde_json::from_str(contents).unwrap();
            assert_eq!(json["properties"]["CanBeDropped"], false);
            assert_eq!(json["properties"]["RequiresHandle"], false);
            assert_eq!(json["properties"]["Enabled"], false);
            assert_eq!(json["properties"]["ManualActivationOnly"], true);
            assert_eq!(json["properties"]["ToolTip"], "Example");
            assert_eq!(json["attributes"]["Alpha"], "kept");
            assert_eq!(json["attributes"]["Zeta"], 3.0);
        }
        if let Some(previous) = &baseline {
            assert_eq!(
                &files, previous,
                "Identical syncback input changed JSON bytes"
            );
        } else {
            baseline = Some(files);
        }
        // Reset the source fixture so every subprocess must serialize the same changes.
        fs::remove_dir_all(&src).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn syncback_init_named_scripts_preserve_their_parent_across_repeated_runs() {
    use crate::rojo_test::io_util::ROJO_PATH;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    use std::{fs, process::Command};

    for (class, names) in [
        (
            "ModuleScript",
            &[
                "Init",
                "init",
                "init.server",
                "init.client",
                "init.plugin",
                "Ordinary",
            ][..],
        ),
        ("Script", &["Init", "Ordinary"][..]),
        ("LocalScript", &["Init", "Ordinary"][..]),
        ("LocalizationTable", &["Init", "Ordinary"][..]),
    ] {
        for name in names {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("src/server")).unwrap();
            let project = dir.path().join("default.project.json");
            fs::write(&project, r#"{"name":"Root","tree":{"$className":"DataModel","ServerScriptService":{"$path":"src/server"}}}"#).unwrap();
            let (property, contents) = if class == "LocalizationTable" {
                (
                    "Contents",
                    r#"[{"key":"hello","source":"Hello","values":{}}]"#,
                )
            } else {
                ("Source", "return 42")
            };
            let model = WeakDom::new(
                InstanceBuilder::new("DataModel").with_child(
                    InstanceBuilder::new("ServerScriptService").with_child(
                        InstanceBuilder::new("Folder")
                            .with_name("Folder")
                            .with_child(
                                InstanceBuilder::new(class)
                                    .with_name(*name)
                                    .with_property(property, contents),
                            ),
                    ),
                ),
            );
            let input = dir.path().join("input.rbxl");
            rbx_binary::to_writer(
                fs::File::create(&input).unwrap(),
                &model,
                model.root().children(),
            )
            .unwrap();
            for pass in 1..=2 {
                let output = Command::new(ROJO_PATH)
                    .arg("syncback")
                    .arg(&project)
                    .arg("--input")
                    .arg(&input)
                    .arg("--non-interactive")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{class} {name} pass={pass}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let output_path = dir.path().join("output.rbxl");
            let output = Command::new(ROJO_PATH)
                .arg("build")
                .arg(&project)
                .arg("-o")
                .arg(&output_path)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            let result = rbx_binary::from_reader(fs::File::open(output_path).unwrap()).unwrap();
            let service = result.get_by_ref(result.root().children()[0]).unwrap();
            let folder = result.get_by_ref(service.children()[0]).unwrap();
            assert_eq!(folder.class.as_str(), "Folder", "{class} {name}");
            assert_eq!(folder.children().len(), 1, "{class} {name}");
            let script = result.get_by_ref(folder.children()[0]).unwrap();
            assert_eq!(script.class.as_str(), class, "{class} {name}");
            assert_eq!(&script.name, name);
            let actual = match script.properties.get(&property.into()).unwrap() {
                rbx_dom_weak::types::Variant::String(value) => value,
                other => panic!("Unexpected {property}: {other:?}"),
            };
            if property == "Contents" {
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(actual).unwrap(),
                    serde_json::from_str::<serde_json::Value>(contents).unwrap()
                );
            } else {
                assert_eq!(actual, contents);
            }
        }
    }
}
