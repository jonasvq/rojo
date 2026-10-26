use librojo::{
    build_project, write_project, AmbiguousValue, BuildFormat, Project, UnresolvedValue,
};
use memofs::{InMemoryFs, Vfs, VfsSnapshot};
use rbx_dom_weak::types::Variant;

fn loaded_project() -> (Vfs, Project) {
    let mut fs = InMemoryFs::new();
    fs.load_snapshot(
        "/project/default.project.json",
        VfsSnapshot::file(r#"{"name":"Root","tree":{"$className":"Folder","$path":"source"}}"#),
    )
    .unwrap();
    fs.load_snapshot(
        "/project/source",
        VfsSnapshot::dir([("child.lua", VfsSnapshot::file("return 42"))]),
    )
    .unwrap();
    let vfs = Vfs::new(fs);
    let project = Project::load_exact(
        &vfs,
        std::path::Path::new("/project/default.project.json"),
        None,
    )
    .unwrap();
    (vfs, project)
}

#[test]
fn invalid_loaded_project_has_context_and_does_not_poison_later_builds() {
    let (vfs, project) = loaded_project();
    let mut invalid = project.clone();
    invalid.name = None;
    let error = format!("{:#}", build_project(&vfs, &invalid).unwrap_err());
    assert!(error.contains("missing a name"), "{error}");
    assert!(error.contains("default.project.json"), "{error}");
    invalid = project.clone();
    invalid.file_location = "/missing.project.json".into();
    let error = format!("{:#}", build_project(&vfs, &invalid).unwrap_err());
    assert!(error.contains("missing.project.json"), "{error}");
    invalid.file_location = "/project/source".into();
    let error = format!("{:#}", build_project(&vfs, &invalid).unwrap_err());
    assert!(error.contains("must be a file"), "{error}");
    let tree = build_project(&vfs, &project).unwrap();
    assert_eq!(
        tree.get_instance(tree.get_root_id())
            .unwrap()
            .children()
            .len(),
        1
    );
}

#[test]
fn loaded_root_is_not_reparsed_but_nested_project_cycles_are_checked() {
    let (vfs, mut project) = loaded_project();
    vfs.write("/project/default.project.json", b"invalid JSON")
        .unwrap();
    assert!(build_project(&vfs, &project).is_ok());
    project.tree.path = Some(librojo::PathNode::Required("default.project.json".into()));
    let error = format!("{:#}", build_project(&vfs, &project).unwrap_err());
    assert!(error.contains("Circular project reference"), "{error}");
}

#[test]
fn writer_errors_propagate_and_success_does_not_flush() {
    struct Writer {
        fail: bool,
        flushed: bool,
    }
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail {
                Err(std::io::Error::other("consumer writer failed"))
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }
    let (vfs, project) = loaded_project();
    let tree = build_project(&vfs, &project).unwrap();
    for format in [
        BuildFormat::Rbxm,
        BuildFormat::Rbxl,
        BuildFormat::Rbxmx,
        BuildFormat::Rbxlx,
    ] {
        let mut writer = Writer {
            fail: true,
            flushed: false,
        };
        let error = format!(
            "{:#}",
            write_project(&mut writer, &tree, format).unwrap_err()
        );
        assert!(error.contains("consumer writer failed"), "{error}");
        writer.fail = false;
        write_project(&mut writer, &tree, format).unwrap();
        assert!(!writer.flushed);
    }
}

#[test]
fn loaded_project_builds_and_serializes_without_a_cli_binary() {
    let mut fs = InMemoryFs::new();
    fs.load_snapshot(
        "/project/default.project.json",
        VfsSnapshot::file(
            r#"{
        "name":"Root","tree":{"$className":"Folder",
            "Target":{"$className":"Folder","$attributes":{"Rojo_Id":"TARGET"}},
            "Link":{"$className":"ObjectValue","$attributes":{"Rojo_Target_Value":"TARGET"}},
            "Value":{"$className":"IntValue","$properties":{"Value":1}}
        }}"#,
        ),
    )
    .unwrap();
    let vfs = Vfs::new(fs);
    let mut project = Project::load_exact(
        &vfs,
        std::path::Path::new("/project/default.project.json"),
        None,
    )
    .unwrap();
    project
        .tree
        .children
        .get_mut("Value")
        .unwrap()
        .properties
        .insert(
            "Value".into(),
            UnresolvedValue::Ambiguous(AmbiguousValue::Number(42.0)),
        );
    let tree = build_project(&vfs, &project).unwrap();
    for format in [
        BuildFormat::Rbxm,
        BuildFormat::Rbxl,
        BuildFormat::Rbxmx,
        BuildFormat::Rbxlx,
    ] {
        let mut output = Vec::new();
        write_project(&mut output, &tree, format).unwrap();
        let dom = if matches!(format, BuildFormat::Rbxm | BuildFormat::Rbxl) {
            rbx_binary::from_reader(output.as_slice()).unwrap()
        } else {
            rbx_xml::from_reader(output.as_slice(), rbx_xml::DecodeOptions::new()).unwrap()
        };
        let root = if matches!(format, BuildFormat::Rbxm | BuildFormat::Rbxmx) {
            dom.get_by_ref(dom.root().children()[0]).unwrap()
        } else {
            dom.root()
        };
        let find = |name: &str| {
            root.children()
                .iter()
                .map(|id| dom.get_by_ref(*id).unwrap())
                .find(|instance| instance.name == name)
                .unwrap()
        };
        assert_eq!(
            find("Value").properties.get(&"Value".into()),
            Some(&Variant::Int64(42))
        );
        assert_eq!(
            find("Link").properties.get(&"Value".into()),
            Some(&Variant::Ref(find("Target").referent()))
        );
    }
}
