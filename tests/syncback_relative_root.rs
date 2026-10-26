use librojo::{build_project, syncback_loop, Project};
use memofs::Vfs;
use rbx_dom_weak::{InstanceBuilder, WeakDom};

#[test]
fn relative_syncback_directory_is_resolved_from_project_not_process_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("unique-relative-destination");
    std::fs::create_dir(&source).unwrap();
    let path = dir.path().join("default.project.json");
    std::fs::write(
        &path,
        r#"{"name":"Root","tree":{"$path":"unique-relative-destination"}}"#,
    )
    .unwrap();
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = Project::load_exact(&vfs, &path, None).unwrap();
    let mut tree = build_project(&vfs, &project).unwrap();
    let new = WeakDom::new(
        InstanceBuilder::new("Folder").with_name("Root").with_child(
            InstanceBuilder::new("ModuleScript")
                .with_name("Example")
                .with_property("Source", "return 42"),
        ),
    );
    let snapshot = syncback_loop(&vfs, &mut tree, new, &project).unwrap();
    assert!(
        snapshot
            .added_files()
            .iter()
            .any(|path| path.ends_with("unique-relative-destination/Example.luau")),
        "relative source directory lost its new script: {:?}",
        snapshot.added_files()
    );
}
