use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context};
use memofs::Vfs;

use crate::{
    snapshot::InstigatingSource, variant_eq::variant_eq, InstanceContext, InstanceSnapshot,
    MultiplePaths, PathNode, ProjectNode,
};

use super::{snapshot_from_vfs, Middleware};

pub(super) fn snapshot_paths(
    context: &InstanceContext,
    vfs: &Vfs,
    project: &Path,
    name: &str,
    paths: &MultiplePaths,
) -> anyhow::Result<InstanceSnapshot> {
    ensure!(context.sourcemap_only,
        "$path arrays are supported only by sourcemap; build, serve and syncback require a single source. Project: {}", project.display());
    let mut sources = Vec::new();
    let mut merged = None;
    for path in paths.paths() {
        let path = project
            .parent()
            .context("Project has no parent directory")?
            .join(path);
        let path = vfs.canonicalize(&path).with_context(|| {
            format!(
                "Could not resolve array source {} in {}",
                path.display(),
                project.display()
            )
        })?;
        if sources.contains(&path) {
            continue;
        }
        ensure!(
            vfs.metadata(&path)?.is_dir(),
            "Array source must be a directory: {}",
            path.display()
        );
        ensure!(super::get_dir_middleware(vfs, &path)?.0 == Middleware::Dir,
            "Array source must use plain directory middleware, without init scripts or projects: {}", path.display());
        let mut snapshot = snapshot_from_vfs(context, vfs, &path)?.with_context(|| {
            format!(
                "Array source did not produce an instance: {}",
                path.display()
            )
        })?;
        validate_directory_children(&snapshot)?;
        snapshot.name = name.to_owned().into();
        if let Some(merged) = &mut merged {
            merge(merged, snapshot, project)?;
        } else {
            merged = Some(snapshot);
        }
        sources.push(path);
    }
    let mut merged = merged.expect("MultiplePaths is nonempty");
    set_source(&mut merged, sources, project);
    Ok(merged)
}

fn validate_directory_children(snapshot: &InstanceSnapshot) -> anyhow::Result<()> {
    if snapshot.metadata.middleware != Some(Middleware::Dir) {
        return Ok(());
    }
    let mut names = HashMap::new();
    for child in &snapshot.children {
        ensure!(
            names.insert(child.name.as_ref(), child).is_none(),
            "Ambiguous array directory child {} under {:?}",
            child.name,
            snapshot.metadata.instigating_source
        );
        validate_directory_children(child)?;
    }
    Ok(())
}

fn sources(snapshot: &InstanceSnapshot) -> Vec<PathBuf> {
    match snapshot.metadata.instigating_source.as_ref() {
        Some(InstigatingSource::Path(path)) => vec![path.clone()],
        Some(InstigatingSource::ProjectNode { node, .. }) => node
            .path
            .as_ref()
            .map(|path| path.paths().to_vec())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

fn set_source(snapshot: &mut InstanceSnapshot, paths: Vec<PathBuf>, project: &Path) {
    snapshot.metadata.instigating_source = Some(InstigatingSource::ProjectNode {
        path: project.to_owned(),
        name: snapshot.name.to_string(),
        node: Box::new(ProjectNode {
            path: Some(PathNode::Multiple(MultiplePaths::new(paths).unwrap())),
            ..ProjectNode::default()
        }),
        // Synthetic containers preserve their filesystem class, without service inference.
        parent_class: None,
    });
}

fn merge(
    left: &mut InstanceSnapshot,
    right: InstanceSnapshot,
    project: &Path,
) -> anyhow::Result<()> {
    let mut paths = sources(left);
    let right_paths = sources(&right);
    let describe = || {
        format!(
            "{} and {} (project {})",
            paths
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            right_paths
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            project.display()
        )
    };
    ensure!(
        left.metadata.middleware == Some(Middleware::Dir)
            && right.metadata.middleware == Some(Middleware::Dir),
        "Array child collision at {}: only plain directory containers can merge: {}",
        left.name,
        describe()
    );
    ensure!(
        left.class_name == right.class_name
            && left.properties.len() == right.properties.len()
            && left.properties.iter().all(|(key, value)| right
                .properties
                .get(key)
                .is_some_and(|other| variant_eq(value, other)))
            && left.metadata.context == right.metadata.context
            && left.metadata.ignore_unknown_instances == right.metadata.ignore_unknown_instances
            && left.metadata.specified_id == right.metadata.specified_id
            && left.metadata.schema == right.metadata.schema,
        "Conflicting array directory metadata at {}: {}",
        left.name,
        describe()
    );
    let mut positions: HashMap<String, usize> = left
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| (child.name.to_string(), index))
        .collect();
    for child in right.children {
        if let Some(index) = positions.get(child.name.as_ref()) {
            merge(&mut left.children[*index], child, project)?;
        } else {
            positions.insert(child.name.to_string(), left.children.len());
            left.children.push(child);
        }
    }
    for path in right.metadata.relevant_paths {
        if !left.metadata.relevant_paths.contains(&path) {
            left.metadata.relevant_paths.push(path);
        }
    }
    paths.extend(right_paths);
    set_source(left, paths, project);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Project;
    use memofs::{InMemoryFs, VfsSnapshot};

    fn fixture(project: &str, a: VfsSnapshot, b: VfsSnapshot) -> (Vfs, Project) {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/project",
            VfsSnapshot::dir([
                ("default.project.json", VfsSnapshot::file(project)),
                ("a", a),
                ("b", b),
            ]),
        )
        .unwrap();
        let vfs = Vfs::new(fs);
        let project =
            Project::load_exact(&vfs, Path::new("/project/default.project.json"), None).unwrap();
        (vfs, project)
    }

    fn snapshot(vfs: &Vfs, project: &Project) -> anyhow::Result<InstanceSnapshot> {
        let context = InstanceContext {
            sourcemap_only: true,
            ..InstanceContext::default()
        };
        super::super::project::snapshot_loaded_project(&context, vfs, project).map(Option::unwrap)
    }

    #[test]
    fn recursive_merge_retains_order_all_sources_and_root_decorations() {
        let (vfs, project) = fixture(
            r#"{"name":"Root","tree":{"$path":["a","./a","b"],"$className":"Model","$attributes":{"Example":true}}}"#,
            VfsSnapshot::dir([(
                "Common",
                VfsSnapshot::dir([("first.lua", VfsSnapshot::file("return 1"))]),
            )]),
            VfsSnapshot::dir([(
                "Common",
                VfsSnapshot::dir([("second.lua", VfsSnapshot::file("return 2"))]),
            )]),
        );
        let result = snapshot(&vfs, &project).unwrap();
        assert_eq!(result.class_name, "Model");
        assert!(result.properties.contains_key(&"Attributes".into()));
        assert_eq!(result.children.len(), 1);
        let common = &result.children[0];
        assert_eq!(
            common
                .children
                .iter()
                .map(|child| child.name.as_ref())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        let Some(InstigatingSource::ProjectNode { node, .. }) = &common.metadata.instigating_source
        else {
            panic!("merged source missing")
        };
        let paths = node.path.as_ref().unwrap().paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|path| path.has_root()));
        for path in paths {
            assert!(common.metadata.relevant_paths.contains(path));
        }
        assert_eq!(common.metadata.middleware, Some(Middleware::Dir));
    }

    #[test]
    fn collision_reports_both_sources_without_leaf_precedence() {
        let (vfs, project) = fixture(
            r#"{"name":"Root","tree":{"$path":["a","b"]}}"#,
            VfsSnapshot::dir([("same.lua", VfsSnapshot::file("return 1"))]),
            VfsSnapshot::dir([("same.luau", VfsSnapshot::file("return 2"))]),
        );
        let error = format!("{:#}", snapshot(&vfs, &project).unwrap_err());
        assert!(
            error.contains("collision")
                && error.contains("same.lua")
                && error.contains("same.luau"),
            "{error}"
        );
    }

    #[test]
    fn directory_metadata_must_agree_but_matching_custom_classes_merge() {
        for (right, succeeds) in [
            (
                r#"{"className":"Model","properties":{"Archivable":false}}"#,
                true,
            ),
            (
                r#"{"className":"Model","properties":{"Archivable":true}}"#,
                false,
            ),
            (
                r#"{"className":"Folder","properties":{"Archivable":false}}"#,
                false,
            ),
        ] {
            let (vfs, project) = fixture(
                r#"{"name":"Root","tree":{"$path":["a","b"]}}"#,
                VfsSnapshot::dir([(
                    "init.meta.json",
                    VfsSnapshot::file(r#"{"className":"Model","properties":{"Archivable":false}}"#),
                )]),
                VfsSnapshot::dir([("init.meta.json", VfsSnapshot::file(right))]),
            );
            let result = snapshot(&vfs, &project);
            assert_eq!(result.is_ok(), succeeds, "{result:?}");
            if let Err(error) = result {
                let text = format!("{error:#}");
                assert!(
                    text.contains("metadata") && text.contains('a') && text.contains('b'),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn rejects_non_directory_roots_init_roots_missing_paths_and_explicit_child_collisions() {
        for root in [
            VfsSnapshot::file("return 1"),
            VfsSnapshot::dir([("init.lua", VfsSnapshot::file("return 1"))]),
            VfsSnapshot::dir([(
                "default.project.json",
                VfsSnapshot::file(r#"{"name":"Other","tree":{"$className":"Folder"}}"#),
            )]),
        ] {
            let (vfs, project) = fixture(
                r#"{"name":"Root","tree":{"$path":["a","b"]}}"#,
                root,
                VfsSnapshot::empty_dir(),
            );
            assert!(snapshot(&vfs, &project).is_err());
        }
        let (vfs, mut project) = fixture(
            r#"{"name":"Root","tree":{"$path":["a","b"],"same":{"$className":"Folder"}}}"#,
            VfsSnapshot::dir([("same", VfsSnapshot::empty_dir())]),
            VfsSnapshot::empty_dir(),
        );
        let error = format!("{:#}", snapshot(&vfs, &project).unwrap_err());
        assert!(error.contains("Explicit project child same"), "{error}");
        project.tree.children.clear();
        project.tree.path = Some(PathNode::Multiple(
            MultiplePaths::new(vec!["missing".into()]).unwrap(),
        ));
        let error = format!("{:#}", snapshot(&vfs, &project).unwrap_err());
        assert!(error.contains("missing"), "{error}");
    }

    #[test]
    fn normal_build_rejects_arrays_and_single_paths_still_work() {
        let (vfs, mut project) = fixture(
            r#"{"name":"Root","tree":{"$path":["a","b"]}}"#,
            VfsSnapshot::empty_dir(),
            VfsSnapshot::empty_dir(),
        );
        let error = format!("{:#}", crate::build_project(&vfs, &project).unwrap_err());
        assert!(error.contains("only by sourcemap"), "{error}");
        let mut tree = crate::RojoTree::new(InstanceSnapshot::new());
        let new_dom = rbx_dom_weak::WeakDom::new(rbx_dom_weak::InstanceBuilder::new("Folder"));
        let error = format!(
            "{:#}",
            crate::syncback_loop(&vfs, &mut tree, new_dom, &project)
                .err()
                .unwrap()
        );
        assert!(error.contains("cannot be used for syncback"), "{error}");
        project.tree.path = Some(PathNode::Required("a".into()));
        assert!(crate::build_project(&vfs, &project).is_ok());
    }

    #[test]
    fn multiple_paths_validate_nonempty_and_preserve_primary_and_serialization() {
        for invalid in ["[]", "[null]", "[1]", r#"["a",{}]"#] {
            assert!(
                serde_json::from_str::<PathNode>(invalid).is_err(),
                "{invalid}"
            );
        }
        assert!(MultiplePaths::new(Vec::new()).is_err());
        assert!(matches!(
            serde_json::from_str::<PathNode>(r#"["a"]"#).unwrap(),
            PathNode::Multiple(_)
        ));
        let node: PathNode = serde_json::from_str(r#"["a","b"]"#).unwrap();
        assert_eq!(node.path(), Path::new("a"));
        assert_eq!(node.paths().len(), 2);
        assert!(!node.is_optional());
        assert_eq!(serde_json::to_string(&node).unwrap(), r#"["a","b"]"#);
    }
}
