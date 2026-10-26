use std::io::Write;

use anyhow::{ensure, Context};
use memofs::Vfs;

use crate::{
    snapshot::{apply_patch_set, compute_patch_set},
    snapshot_middleware::snapshot_loaded_project,
    InstanceContext, InstanceSnapshot, Project, RojoTree,
};

/// The serialized model or place format used by [`write_project`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildFormat {
    /// A binary model, including the project root.
    Rbxm,
    /// A binary place, containing the project root's children.
    Rbxl,
    /// An XML model, including the project root.
    Rbxmx,
    /// An XML place, containing the project root's children.
    Rbxlx,
}

/// Build a loaded project, including caller edits, without starting a server.
///
/// The project must have a name and an existing VFS file origin. Relative
/// sources resolve from that origin; the root project's bytes are not reloaded.
/// The VFS watch configuration is left unchanged.
pub fn build_project(vfs: &Vfs, project: &Project) -> anyhow::Result<RojoTree> {
    Ok(build_snapshot(project_snapshot(vfs, project)?))
}

pub(crate) fn project_snapshot(vfs: &Vfs, project: &Project) -> anyhow::Result<InstanceSnapshot> {
    let mut project = project.clone();
    project.file_location = vfs.canonicalize(&project.file_location).with_context(|| {
        format!(
            "Could not resolve project origin {}",
            project.file_location.display()
        )
    })?;
    ensure!(
        vfs.metadata(&project.file_location)?.is_file(),
        "Project origin must be a file: {}",
        project.file_location.display()
    );
    let context =
        InstanceContext::with_emit_legacy_scripts(Some(project.effective_emit_legacy_scripts()));
    snapshot_loaded_project(&context, vfs, &project)
        .with_context(|| {
            format!(
                "Could not build project {}",
                project.file_location.display()
            )
        })?
        .context("Project did not produce a root instance")
}

#[cfg(feature = "cli")]
pub(crate) fn stdin_snapshot(vfs: &Vfs, project: &Project) -> anyhow::Result<InstanceSnapshot> {
    let mut context =
        InstanceContext::with_emit_legacy_scripts(Some(project.effective_emit_legacy_scripts()));
    context.offline_session = true;
    crate::snapshot_middleware::snapshot_stdin_project(&context, vfs, project)?
        .context("stdin project did not produce a root instance")
}

pub(crate) fn build_snapshot(snapshot: InstanceSnapshot) -> RojoTree {
    // Initial patch application also resolves references against the final tree.
    let mut tree = RojoTree::new(InstanceSnapshot::new());
    let patch = compute_patch_set(Some(snapshot), &tree, tree.get_root_id());
    apply_patch_set(&mut tree, patch);
    tree
}

/// Serialize a built project to a caller-owned writer without flushing it.
pub fn write_project<W: Write>(
    writer: W,
    tree: &RojoTree,
    format: BuildFormat,
) -> anyhow::Result<()> {
    write_project_with_compression(writer, tree, format, rbx_binary::CompressionType::default())
}

pub(crate) fn write_project_with_compression<W: Write>(
    writer: W,
    tree: &RojoTree,
    format: BuildFormat,
    compression: rbx_binary::CompressionType,
) -> anyhow::Result<()> {
    let root = tree.get_root_id();
    if matches!(format, BuildFormat::Rbxl | BuildFormat::Rbxlx) {
        // Inspect the built tree: a $path/model may supply either container.
        // Creating one implicitly here would change authored project contents.
        for node in tree
            .descendants(root)
            .filter(|node| node.class_name() == "StarterPlayer")
        {
            let has_container = |class_name: &str| {
                node.children().iter().any(|id| {
                    tree.get_instance(*id).is_some_and(|child| {
                        child.class_name() == class_name && child.name() == class_name
                    })
                })
            };
            if has_container("StarterCharacterScripts") && !has_container("StarterPlayerScripts") {
                log::warn!("{} contains StarterCharacterScripts without StarterPlayerScripts. Declare both containers explicitly in the project or its source model when building a place; see docs/fully-managed-workflow.md#starterplayer-containers.", tree.inner().full_path_of(node.id(), "/"));
            }
        }
    }
    let model_ids = [root];
    let ids = match format {
        BuildFormat::Rbxm | BuildFormat::Rbxmx => &model_ids,
        BuildFormat::Rbxl | BuildFormat::Rbxlx => tree.get_instance(root).unwrap().children(),
    };
    match format {
        BuildFormat::Rbxm | BuildFormat::Rbxl => {
            crate::binary_serialization::to_writer_with_compression(
                writer,
                tree.inner(),
                ids,
                compression,
            )?;
        }
        BuildFormat::Rbxmx | BuildFormat::Rbxlx => {
            let options = rbx_xml::EncodeOptions::new()
                .property_behavior(rbx_xml::EncodePropertyBehavior::WriteUnknown);
            rbx_xml::to_writer(writer, tree.inner(), ids, options)?;
        }
    }
    Ok(())
}
