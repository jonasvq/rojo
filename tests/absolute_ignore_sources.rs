use librojo::{build_project, Project};
use memofs::Vfs;
use std::path::Path;

fn child_names(project_path: &Path) -> Vec<String> {
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = Project::load_exact(&vfs, project_path, None).unwrap();
    let tree = build_project(&vfs, &project).unwrap();
    let shared = tree
        .inner()
        .get_by_ref(tree.inner().root().children()[0])
        .unwrap();
    let mut names: Vec<_> = shared
        .children()
        .iter()
        .map(|id| tree.inner().get_by_ref(*id).unwrap().name.clone())
        .collect();
    names.sort();
    names
}

fn write_source(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    for name in ["ignored.lua", "keep.lua", "kept.luau"] {
        std::fs::write(path.join(name), "return 1").unwrap();
    }
}

fn write_project(path: &Path, source: &Path, rules: &[&str]) {
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "name":"Root", "globIgnorePaths":rules,
            "tree":{"$className":"Folder","Shared":{"$path":source}}
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn absolute_and_relative_bindings_apply_the_same_project_ignore_and_negation_rules() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let shared = dir.path().join("shared");
    write_source(&shared);
    let path = config.join("default.project.json");
    for source in [Path::new("../shared"), shared.as_path()] {
        write_project(&path, source, &["**/*.lua", "!**/keep.lua"]);
        assert_eq!(
            child_names(&path),
            ["keep", "kept"],
            "source {}",
            source.display()
        );
        write_project(&path, source, &["../shared/*.lua", "!../shared/keep.lua"]);
        assert_eq!(
            child_names(&path),
            ["keep", "kept"],
            "scoped source {}",
            source.display()
        );
    }
}

#[cfg(windows)]
#[test]
fn cross_drive_sources_apply_wildcard_ignores_and_negations() {
    let dir = tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
    let shared = tempfile::tempdir().unwrap();
    write_source(shared.path());
    if dir.path().components().next() == shared.path().components().next() {
        // Same-drive behavior is already covered; this environment has no
        // second drive available through its configured temporary directory.
        return;
    }
    let path = dir.path().join("default.project.json");
    write_project(&path, shared.path(), &["**/*.lua", "!**/keep.lua"]);
    assert_eq!(child_names(&path), ["keep", "kept"]);
}
