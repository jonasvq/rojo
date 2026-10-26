mod place_update;
mod stdin;

use std::{
    io::{BufWriter, Write},
    mem::forget,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context};
use clap::{CommandFactory, Parser, ValueEnum};
use fs_err::File;
use memofs::Vfs;
use roblox_install::RobloxStudio;
use tokio::runtime::Runtime;

use crate::serve_session::ServeSession;

use super::resolve_project_path;

const UNKNOWN_OUTPUT_KIND_ERR: &str = "Could not detect what kind of file to build. \
                                       Expected output file to end in .rbxl, .rbxlx, .rbxm, or .rbxmx.";
const UNKNOWN_PLUGIN_KIND_ERR: &str = "Could not detect what kind of file to build. \
                                       Expected plugin file to end in .rbxm or .rbxmx.";

/// Compression for binary build outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BuildCompression {
    Lz4,
    Zstd,
    None,
}

impl From<BuildCompression> for rbx_binary::CompressionType {
    fn from(value: BuildCompression) -> Self {
        match value {
            BuildCompression::Lz4 => Self::Lz4,
            BuildCompression::Zstd => Self::Zstd,
            BuildCompression::None => Self::None,
        }
    }
}

/// Generates a model or place file from the Rojo project.
#[derive(Debug, Parser)]
pub struct BuildCommand {
    /// Path to the project to build, or - to read project JSON from stdin.
    /// Defaults to the current directory.
    #[clap(default_value = "")]
    pub project: PathBuf,

    /// Where to output the result.
    ///
    /// Should end in .rbxm, .rbxl, .rbxmx, or .rbxlx.
    #[clap(long, short, conflicts_with = "plugin")]
    pub output: Option<PathBuf>,

    /// Alternative to the output flag that outputs the result in the local plugins folder.
    ///
    /// Should end in .rbxm or .rbxl.
    #[clap(long, short, conflicts_with = "output")]
    pub plugin: Option<PathBuf>,

    /// Whether to automatically rebuild when any input files change.
    #[clap(long)]
    pub watch: bool,

    /// Update an existing place, preserving objects outside the project scopes.
    #[clap(long, requires = "output", conflicts_with_all = &["watch", "plugin"])]
    pub input: Option<PathBuf>,

    /// Compression for binary output. Omission retains LZ4; XML rejects this option.
    #[clap(long, value_enum)]
    pub compress: Option<BuildCompression>,
}

impl BuildCommand {
    pub fn run(self) -> anyhow::Result<()> {
        if let Some(input) = &self.input {
            return place_update::run(&self, input);
        }
        let (output_path, output_kind) = match (self.output, self.plugin) {
            (None, None) => {
                BuildCommand::command()
                    .error(
                        clap::ErrorKind::MissingRequiredArgument,
                        "one of the following arguments must be provided: \n    --output <OUTPUT>\n    --plugin <PLUGIN>",
                    )
                    .exit();
            }
            (Some(output), None) => {
                let output_kind =
                    OutputKind::from_output_path(&output).context(UNKNOWN_OUTPUT_KIND_ERR)?;

                (output, output_kind)
            }
            (None, Some(plugin)) => {
                if plugin.is_absolute() {
                    bail!("plugin flag path cannot be absolute.")
                }

                let output_kind =
                    OutputKind::from_plugin_path(&plugin).context(UNKNOWN_PLUGIN_KIND_ERR)?;
                let studio = RobloxStudio::locate()?;

                (studio.plugins_path().join(&plugin), output_kind)
            }
            _ => unreachable!(),
        };

        validate_compression(self.compress, output_kind)?;
        let compression = self.compress.map(Into::into).unwrap_or_default();
        #[cfg(windows)]
        validate_output_path(&output_path)?;

        log::trace!("Constructing in-memory filesystem");
        let vfs = Vfs::new_default()?;
        vfs.set_watch_enabled(self.watch);

        let session = if stdin::is_stdin(&self.project) {
            let project = stdin::read_project(&vfs)?;
            ServeSession::new_from_stdin(vfs, project).context("Could not build stdin project")?
        } else {
            ServeSession::new_for_offline(vfs, resolve_project_path(&self.project)?)?
        };
        let mut cursor = session.message_queue().cursor();

        write_model(&session, &output_path, output_kind, compression)?;

        if self.watch {
            let rt = Runtime::new().context("Failed to start the async runtime for watch mode")?;

            loop {
                let receiver = session.message_queue().subscribe(cursor);
                let (new_cursor, _patch_set) = match rt.block_on(receiver) {
                    Ok(message) => message,
                    // The message queue was dropped, so there is nothing left
                    // to watch. Stop watching gracefully.
                    Err(_) => break,
                };
                cursor = new_cursor;

                // The watcher debounces each path independently. A compiler
                // rewriting many scripts can therefore publish many patches
                // together. Give that batch one bounded collection window;
                // a fixed window also avoids starving builds under continuous
                // edits. Capture the cursor before exporting so later edits
                // still wake the next iteration.
                std::thread::sleep(Duration::from_millis(100));
                cursor = cursor.max(session.message_queue().cursor());

                write_model(&session, &output_path, output_kind, compression)?;
            }
        }

        // Avoid dropping ServeSession: it's potentially VERY expensive to drop
        // and we're about to exit anyways.
        forget(session);

        Ok(())
    }
}

fn validate_compression(
    compress: Option<BuildCompression>,
    kind: OutputKind,
) -> anyhow::Result<()> {
    if compress.is_some() && matches!(kind, OutputKind::Rbxmx | OutputKind::Rbxlx) {
        bail!("--compress requires binary .rbxm or .rbxl output, not XML");
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn validate_output_path(output: &Path) -> anyhow::Result<()> {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("Invalid output file name: {}", output.display()))?;
    crate::syncback::validate_file_name(name)
        .and_then(|_| {
            // Windows device names remain reserved with extensions and without
            // regard to case. Colons otherwise silently create alternate streams.
            let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
            crate::syncback::validate_file_name(stem)
        })
        .with_context(|| format!("Invalid output file name: {}", output.display()))
}

/// The different kinds of output that Rojo can build to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputKind {
    /// An XML model file.
    Rbxmx,

    /// An XML place file.
    Rbxlx,

    /// A binary model file.
    Rbxm,

    /// A binary place file.
    Rbxl,
}

impl OutputKind {
    fn from_output_path(output: &Path) -> Option<OutputKind> {
        let extension = output.extension()?.to_str()?;

        match extension {
            "rbxlx" => Some(OutputKind::Rbxlx),
            "rbxmx" => Some(OutputKind::Rbxmx),
            "rbxl" => Some(OutputKind::Rbxl),
            "rbxm" => Some(OutputKind::Rbxm),
            _ => None,
        }
    }

    fn from_plugin_path(output: &Path) -> Option<OutputKind> {
        let extension = output.extension()?.to_str()?;

        match extension {
            "rbxmx" => Some(OutputKind::Rbxmx),
            "rbxm" => Some(OutputKind::Rbxm),
            _ => None,
        }
    }
}

#[profiling::function]
fn write_model(
    session: &ServeSession,
    output: &Path,
    output_kind: OutputKind,
    compression: rbx_binary::CompressionType,
) -> anyhow::Result<()> {
    let _operation = crate::diagnostic_context::enter("building output", output);
    println!("Building project '{}'", session.project_name());

    let tree = session.tree();

    log::trace!("Opening output file for write");
    let mut file = BufWriter::new(File::create(output)?);

    let format = match output_kind {
        OutputKind::Rbxm => crate::BuildFormat::Rbxm,
        OutputKind::Rbxl => crate::BuildFormat::Rbxl,
        OutputKind::Rbxmx => crate::BuildFormat::Rbxmx,
        OutputKind::Rbxlx => crate::BuildFormat::Rbxlx,
    };
    crate::builder::write_project_with_compression(&mut file, &tree, format, compression)?;

    file.flush()?;

    let filename = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<invalid utf-8>");
    println!("Built project to {}", filename);

    Ok(())
}
