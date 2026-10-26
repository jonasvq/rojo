use std::{
    borrow::Cow,
    io::{BufWriter, Write},
    mem::forget,
    path::{self, Path, PathBuf},
};

use anyhow::Context;
use clap::Parser;
use fs_err::File;
use memofs::Vfs;
use rayon::prelude::*;
use rbx_dom_weak::{types::Ref, Ustr};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use crate::{
    serve_session::ServeSession,
    snapshot::{AppliedPatchSet, InstanceWithMeta, RojoTree},
};

use super::resolve_project_path;

const PATH_STRIP_FAILED_ERR: &str = "Failed to create relative paths for project file!";
const ABSOLUTE_PATH_FAILED_ERR: &str = "Failed to turn relative path into absolute path!";

/// Representation of a node in the generated sourcemap tree.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcemapNode<'a> {
    name: &'a str,
    class_name: Ustr,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_context_value: Option<u32>,

    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "crate::path_serializer::serialize_vec_absolute"
    )]
    file_paths: Vec<Cow<'a, Path>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<SourcemapNode<'a>>,
}

/// Generates a sourcemap file from the Rojo project.
#[derive(Debug, Parser)]
pub struct SourcemapCommand {
    /// Path to the project to use for the sourcemap. Defaults to the current
    /// directory.
    #[clap(default_value = "")]
    pub project: PathBuf,

    /// Where to output the sourcemap. Omit this to use stdout instead of
    /// writing to a file.
    ///
    /// Should end in .json.
    #[clap(long, short)]
    pub output: Option<PathBuf>,

    /// If non-script files should be included or not. Defaults to false.
    #[clap(long)]
    pub include_non_scripts: bool,

    /// Whether to automatically recreate a snapshot when any input files change.
    #[clap(long)]
    pub watch: bool,

    /// Whether the sourcemap should use absolute paths instead of relative paths.
    #[clap(long)]
    pub absolute: bool,

    /// Make file paths relative to the output file's directory.
    #[clap(long, requires = "output", conflicts_with = "absolute")]
    pub relative_to_output: bool,

    /// Include reflected RunContext names and raw values on Script nodes.
    #[clap(long)]
    pub include_run_context: bool,
}

impl SourcemapCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let project_path = dunce::canonicalize(resolve_project_path(&self.project)?)?;

        log::trace!("Constructing filesystem with StdBackend");
        let vfs = Vfs::new_default()?;
        vfs.set_watch_enabled(self.watch);

        log::trace!("Setting up session for sourcemap generation");
        let session = ServeSession::new_for_sourcemap(vfs, project_path)?;
        let mut cursor = session.message_queue().cursor();

        let filter = if self.include_non_scripts {
            filter_nothing
        } else {
            filter_non_scripts
        };

        // Pre-build a rayon threadpool with a low number of threads to avoid
        // dynamic creation overhead on systems with a high number of cpus.
        log::trace!("Setting rayon global threadpool");
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get().min(6))
            .build_global()
            .ok();

        log::trace!("Writing initial sourcemap");
        write_sourcemap(
            &session,
            self.output.as_deref(),
            filter,
            self.absolute,
            self.relative_to_output,
            self.include_run_context,
        )?;

        if self.watch {
            log::trace!("Setting up runtime for watch mode");
            let rt = Runtime::new().context("Failed to start the async runtime for watch mode")?;

            loop {
                let receiver = session.message_queue().subscribe(cursor);
                let (new_cursor, patch_set) = match rt.block_on(receiver) {
                    Ok(message) => message,
                    // The message queue was dropped, so there is nothing left
                    // to watch. Stop watching gracefully.
                    Err(_) => break,
                };
                cursor = new_cursor;

                if patch_set_affects_sourcemap(
                    &session,
                    &patch_set,
                    filter,
                    self.include_run_context,
                ) {
                    write_sourcemap(
                        &session,
                        self.output.as_deref(),
                        filter,
                        self.absolute,
                        self.relative_to_output,
                        self.include_run_context,
                    )?;
                }
            }
        }

        // Avoid dropping ServeSession: it's potentially VERY expensive to drop
        // and we're about to exit anyways.
        forget(session);

        Ok(())
    }
}

fn filter_nothing(_instance: &InstanceWithMeta) -> bool {
    true
}

fn filter_non_scripts(instance: &InstanceWithMeta) -> bool {
    matches!(
        instance.class_name().as_str(),
        "Script" | "LocalScript" | "ModuleScript"
    )
}

fn patch_set_affects_sourcemap(
    session: &ServeSession,
    patch_set: &[AppliedPatchSet],
    filter: fn(&InstanceWithMeta) -> bool,
    include_run_context: bool,
) -> bool {
    let tree = session.tree();

    // A sourcemap has probably changed when:
    patch_set.par_iter().any(|set| {
        // 1. An instance was removed, in which case it will no
        // longer exist in the tree and we cant check the filter
        !set.removed.is_empty()
            // 2. A newly added instance passes the filter
            || set.added.iter().any(|referent| {
                let instance = tree
                    .get_instance(*referent)
                    .expect("instance did not exist when updating sourcemap");
                filter(&instance)
            })
            // 3. An existing instance has its class name, name,
            // or file paths changed, and passes the filter
            || set.updated.iter().any(|updated| {
                let changed = updated.changed_class_name.is_some()
                    || updated.changed_name.is_some()
                    || updated.changed_metadata.is_some();
                let context_changed = include_run_context
                    && updated.changed_properties.contains_key(&rbx_dom_weak::ustr("RunContext"));
                if changed || context_changed {
                    let instance = tree
                        .get_instance(updated.id)
                        .expect("instance did not exist when updating sourcemap");
                    filter(&instance) && (changed || instance.class_name() == "Script")
                } else {
                    false
                }
            })
    })
}

fn recurse_create_node<'a>(
    tree: &'a RojoTree,
    referent: Ref,
    project_dir: &Path,
    filter: fn(&InstanceWithMeta) -> bool,
    use_absolute_paths: bool,
    include_run_context: bool,
) -> Option<SourcemapNode<'a>> {
    let instance = tree.get_instance(referent).expect("instance did not exist");

    let children: Vec<_> = instance
        .children()
        .par_iter()
        .filter_map(|&child_id| {
            recurse_create_node(
                tree,
                child_id,
                project_dir,
                filter,
                use_absolute_paths,
                include_run_context,
            )
        })
        .collect();

    // If this object has no children and doesn't pass the filter, it doesn't
    // contain any information we're looking for.
    if children.is_empty() && !filter(&instance) {
        return None;
    }

    let file_paths = instance
        .metadata()
        .relevant_paths
        .iter()
        // Not all paths listed as relevant are guaranteed to exist.
        .filter(|path| path.is_file())
        .map(|path| path.as_path());

    let mut output_file_paths: Vec<Cow<'a, Path>> =
        Vec::with_capacity(instance.metadata().relevant_paths.len());

    if use_absolute_paths {
        // It's somewhat important to note here that `path::absolute` takes in a Path and returns a PathBuf
        for val in file_paths {
            output_file_paths.push(Cow::Owned(
                path::absolute(val).expect(ABSOLUTE_PATH_FAILED_ERR),
            ));
        }
    } else {
        for val in file_paths {
            output_file_paths.push(Cow::from(
                pathdiff::diff_paths(val, project_dir).expect(PATH_STRIP_FAILED_ERR),
            ));
        }
    };

    let (run_context, run_context_value) = if include_run_context {
        script_run_context(&instance)
    } else {
        (None, None)
    };
    Some(SourcemapNode {
        name: instance.name(),
        class_name: instance.class_name(),
        run_context,
        run_context_value,
        file_paths: output_file_paths,
        children,
    })
}

fn script_run_context(instance: &InstanceWithMeta<'_>) -> (Option<String>, Option<u32>) {
    use rbx_dom_weak::types::Variant;
    if instance.class_name() != "Script" {
        return (None, None);
    }
    let database = rbx_reflection_database::get().unwrap();
    let value = instance
        .properties()
        .get(&rbx_dom_weak::ustr("RunContext"))
        .or_else(|| {
            database
                .classes
                .get("Script")?
                .default_properties
                .get("RunContext")
        });
    let Some(Variant::Enum(value)) = value else {
        return (None, None);
    };
    let raw = value.to_u32();
    let name = database.enums.get("RunContext").and_then(|context| {
        context
            .items
            .iter()
            .filter(|(_, value)| **value == raw)
            .map(|(name, _)| name.to_string())
            .min()
    });
    (name, Some(raw))
}

fn write_sourcemap(
    session: &ServeSession,
    output: Option<&Path>,
    filter: fn(&InstanceWithMeta) -> bool,
    use_absolute_paths: bool,
    relative_to_output: bool,
    include_run_context: bool,
) -> anyhow::Result<()> {
    let tree = session.tree();

    let output_base;
    let base = if relative_to_output {
        anyhow::ensure!(
            !use_absolute_paths,
            "--relative-to-output conflicts with --absolute"
        );
        let output = output.context("--relative-to-output requires --output")?;
        let parent = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        output_base = dunce::canonicalize(parent).with_context(|| {
            format!(
                "Could not resolve sourcemap output directory {}",
                parent.display()
            )
        })?;
        output_base.as_path()
    } else {
        session.root_dir()
    };

    let root_node = recurse_create_node(
        &tree,
        tree.get_root_id(),
        base,
        filter,
        use_absolute_paths,
        include_run_context,
    );

    if let Some(output_path) = output {
        let mut file = BufWriter::new(File::create(output_path)?);
        serde_json::to_writer(&mut file, &root_node)?;
        file.flush()?;

        println!("Created sourcemap at {}", output_path.display());
    } else {
        let output = serde_json::to_string(&root_node)?;
        println!("{}", output);
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::cli::sourcemap::SourcemapNode;
    use crate::cli::SourcemapCommand;
    use insta::internals::Content;
    use std::path::Path;

    #[test]
    fn run_context_names_raw_values_defaults_and_non_scripts_are_faithful() {
        use crate::snapshot::{InstanceSnapshot, RojoTree};
        use rbx_dom_weak::types::Enum;
        for (raw, name) in [
            (None, Some("Legacy")),
            (Some(0), Some("Legacy")),
            (Some(1), Some("Server")),
            (Some(2), Some("Client")),
            (Some(3), Some("Plugin")),
            (Some(u32::MAX), None),
        ] {
            for class in ["Script", "LocalScript", "ModuleScript", "Folder"] {
                let mut snapshot = InstanceSnapshot::new().name("Node").class_name(class);
                if let Some(raw) = raw {
                    snapshot = snapshot.property("RunContext", Enum::from_u32(raw));
                }
                let tree = RojoTree::new(snapshot);
                for enabled in [false, true] {
                    let node = super::recurse_create_node(
                        &tree,
                        tree.get_root_id(),
                        Path::new("/project"),
                        super::filter_nothing,
                        false,
                        enabled,
                    )
                    .unwrap();
                    let value = serde_json::to_value(node).unwrap();
                    if enabled && class == "Script" {
                        assert_eq!(value["runContextValue"], raw.unwrap_or(0));
                        assert_eq!(value["runContext"].as_str(), name);
                    } else {
                        assert!(value.get("runContext").is_none());
                        assert!(value.get("runContextValue").is_none());
                    }
                    assert_eq!(value["className"], class);
                }
            }
        }
    }

    #[test]
    fn output_relative_cli_requires_output_and_rejects_absolute() {
        use clap::Parser;
        assert!(SourcemapCommand::try_parse_from(["sourcemap", "--relative-to-output"]).is_err());
        assert!(SourcemapCommand::try_parse_from([
            "sourcemap",
            "--relative-to-output",
            "--output",
            "map.json",
            "--absolute"
        ])
        .is_err());
    }

    #[test]
    fn output_relative_reports_missing_parent_without_creating_output() {
        use clap::Parser;
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("default.project.json");
        fs_err::write(
            &project,
            r#"{"name":"Root","tree":{"$className":"Folder"}}"#,
        )
        .unwrap();
        let output = dir.path().join("missing/sourcemap.json");
        let command = SourcemapCommand::try_parse_from([
            "sourcemap",
            project.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--relative-to-output",
        ])
        .unwrap();
        let error = command.run().unwrap_err().to_string();
        assert!(
            error.contains("sourcemap output directory") && error.contains("missing"),
            "{error}"
        );
        assert!(!output.exists());
    }

    #[test]
    fn maps_paths_relative_to_output_only_when_requested() {
        use clap::Parser;
        let dir = tempfile::tempdir().unwrap();
        fs_err::create_dir(dir.path().join("src")).unwrap();
        fs_err::create_dir(dir.path().join("maps")).unwrap();
        fs_err::write(dir.path().join("src/client.client.luau"), "print('client')").unwrap();
        let project = dir.path().join("default.project.json");
        fs_err::write(&project, r#"{"name":"Root","tree":{"$path":"src"}}"#).unwrap();
        let output = dir.path().join("maps/sourcemap.json");
        let command = SourcemapCommand::try_parse_from([
            "sourcemap",
            project.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--relative-to-output",
        ])
        .unwrap();
        command.run().unwrap();
        let map: serde_json::Value =
            serde_json::from_str(&fs_err::read_to_string(output).unwrap()).unwrap();
        assert_eq!(
            map["children"][0]["filePaths"][0],
            "../src/client.client.luau"
        );
    }

    #[test]
    fn exposes_client_run_context_without_changing_script_class() {
        use clap::Parser;
        let dir = tempfile::tempdir().unwrap();
        fs_err::write(dir.path().join("client.client.luau"), "print('client')").unwrap();
        let project = dir.path().join("default.project.json");
        fs_err::write(
            &project,
            r#"{"name":"Client","emitLegacyScripts":false,"tree":{"$path":"client.client.luau"}}"#,
        )
        .unwrap();
        let output = dir.path().join("sourcemap.json");
        let command = SourcemapCommand::try_parse_from([
            "sourcemap",
            project.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--include-run-context",
        ])
        .unwrap();
        command.run().unwrap();
        let map: serde_json::Value =
            serde_json::from_str(&fs_err::read_to_string(output).unwrap()).unwrap();
        assert_eq!(map["className"], "Script");
        assert_eq!(map["runContext"], "Client");
        assert_eq!(map["runContextValue"], 2);
    }

    #[cfg(windows)]
    #[test]
    fn maps_cross_drive_sources_without_panicking_or_changing_relative_paths() {
        use crate::snapshot::{InstanceMetadata, InstanceSnapshot, RojoTree};
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("shared.luau");
        fs_err::write(&source, "return 1").unwrap();
        let tree = RojoTree::new(
            InstanceSnapshot::new()
                .name("Shared")
                .class_name("ModuleScript")
                .metadata(InstanceMetadata::new().relevant_paths(vec![source.clone()])),
        );
        let cross_drive = if source.starts_with("Z:\\") {
            Path::new("Y:\\project")
        } else {
            Path::new("Z:\\project")
        };
        let cross = super::recurse_create_node(
            &tree,
            tree.get_root_id(),
            cross_drive,
            super::filter_nothing,
            false,
            false,
        )
        .unwrap();
        assert_eq!(cross.file_paths[0].as_ref(), source);
        let same = super::recurse_create_node(
            &tree,
            tree.get_root_id(),
            &dir.path().join("project"),
            super::filter_nothing,
            false,
            false,
        )
        .unwrap();
        assert_eq!(same.file_paths[0].as_ref(), Path::new("../shared.luau"));
        let absolute = super::recurse_create_node(
            &tree,
            tree.get_root_id(),
            cross_drive,
            super::filter_nothing,
            true,
            false,
        )
        .unwrap();
        assert_eq!(absolute.file_paths[0].as_ref(), source);
    }

    #[test]
    fn maps_relative_paths() {
        let sourcemap_dir = tempfile::tempdir().unwrap();
        let sourcemap_output = sourcemap_dir.path().join("sourcemap.json");
        let project_path = fs_err::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("test-projects")
                .join("relative_paths")
                .join("project"),
        )
        .unwrap();
        let sourcemap_command = SourcemapCommand {
            project: project_path,
            output: Some(sourcemap_output.clone()),
            include_non_scripts: false,
            watch: false,
            absolute: false,
            relative_to_output: false,
            include_run_context: false,
        };
        assert!(sourcemap_command.run().is_ok());

        let raw_sourcemap_contents = fs_err::read_to_string(sourcemap_output.as_path()).unwrap();
        let sourcemap_contents =
            serde_json::from_str::<SourcemapNode>(&raw_sourcemap_contents).unwrap();
        insta::assert_json_snapshot!(sourcemap_contents);
    }

    #[test]
    fn maps_absolute_paths() {
        let sourcemap_dir = tempfile::tempdir().unwrap();
        let sourcemap_output = sourcemap_dir.path().join("sourcemap.json");
        let project_path = fs_err::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("test-projects")
                .join("relative_paths")
                .join("project"),
        )
        .unwrap();
        let sourcemap_command = SourcemapCommand {
            project: project_path,
            output: Some(sourcemap_output.clone()),
            include_non_scripts: false,
            watch: false,
            absolute: true,
            relative_to_output: false,
            include_run_context: false,
        };
        assert!(sourcemap_command.run().is_ok());

        let raw_sourcemap_contents = fs_err::read_to_string(sourcemap_output.as_path()).unwrap();
        let sourcemap_contents =
            serde_json::from_str::<SourcemapNode>(&raw_sourcemap_contents).unwrap();
        insta::assert_json_snapshot!(sourcemap_contents, {
            ".**.filePaths" => insta::dynamic_redaction(|mut value, _path| {
                let mut paths_count = 0;

                match value {
                    Content::Seq(ref mut vec) => {
                        for path in vec.iter().map(|i| i.as_str().unwrap()) {
                            assert_eq!(fs_err::canonicalize(path).is_ok(), true, "path was not valid");
                            assert_eq!(Path::new(path).is_absolute(), true, "path was not absolute");

                            paths_count += 1;
                        }
                    }
                    _ => panic!("Expected filePaths to be a sequence"),
                }
                format!("[...{} path{} omitted...]", paths_count, if paths_count != 1 { "s" } else { "" } )
            })
        });
    }
}
