use librojo::{build_project, Project};
use memofs::{InMemoryFs, Vfs, VfsSnapshot};
use rojo_project_types::{ProjectFormatVersion, ProjectNode, UnresolvedValue};

#[test]
fn small_crate_types_have_the_same_identity_and_drive_the_existing_builder() {
    fn to_existing(node: ProjectNode) -> librojo::ProjectNode {
        node
    }
    fn to_small(node: librojo::ProjectNode) -> ProjectNode {
        node
    }
    fn value_identity(value: UnresolvedValue) -> librojo::UnresolvedValue {
        value
    }
    let tree: ProjectNode = serde_json::from_str(r#"{"$className":"Folder","Child":{"$className":"StringValue","$properties":{"Value":"from small crate"}}}"#).unwrap();
    let mut fs = InMemoryFs::new();
    fs.load_snapshot(
        "/project/default.project.json",
        VfsSnapshot::file(r#"{"name":"Root","tree":{"$className":"Folder"}}"#),
    )
    .unwrap();
    let vfs = Vfs::new(fs);
    let mut project = Project::load_exact(
        &vfs,
        std::path::Path::new("/project/default.project.json"),
        None,
    )
    .unwrap();
    project.tree = to_existing(to_small(tree));
    project.format_version = Some(ProjectFormatVersion::V2);
    let value = value_identity(serde_json::from_str("2").unwrap())
        .resolve("Script", "RunContext")
        .unwrap();
    assert_eq!(
        value,
        rbx_dom_weak::types::Variant::Enum(rbx_dom_weak::types::Enum::from_u32(2))
    );
    let built = build_project(&vfs, &project).unwrap();
    let child = built
        .descendants(built.get_root_id())
        .find(|node| node.name() == "Child")
        .unwrap();
    assert_eq!(
        child.properties().get(&"Value".into()),
        Some(&rbx_dom_weak::types::Variant::String(
            "from small crate".into()
        ))
    );
    let paths = rojo_project_types::MultiplePaths::new(vec!["a\\b".into()]).unwrap();
    let old: librojo::PathNode = rojo_project_types::PathNode::Multiple(paths);
    assert_eq!(serde_json::to_string(&old).unwrap(), r#"["a/b"]"#);
    assert!(rojo_project_types::MultiplePaths::new(Vec::new()).is_err());
}
