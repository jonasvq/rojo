use std::{
    io::{self, BufReader, Write as _},
    mem::forget,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{ensure, Context};
use clap::Parser;
use fs_err::File;
use memofs::{StdBackend, Vfs, VfsBackend, VfsEvent};
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

use crate::{
    path_serializer::display_absolute,
    serve_session::ServeSession,
    syncback::{syncback_loop, FsSnapshot},
};

use super::{resolve_path, resolve_project_path, GlobalOptions};

const UNKNOWN_INPUT_KIND_ERR: &str = "Could not detect what kind of file was inputted. \
                                       Expected input file to end in .rbxl, .rbxlx, .rbxm, or .rbxmx.";

/// Performs 'syncback' for the provided project, using the `input` file
/// given.
///
/// Syncback exists to convert Roblox files into a Rojo project automatically.
/// It uses the project.json file provided to traverse the Roblox file passed as
/// to serialize Instances to the file system in a format that Rojo understands.
///
/// To ease programmatic use, this command pipes all normal output to stderr.
#[derive(Debug, Parser)]
#[clap(group(clap::ArgGroup::new("watch_consent").args(&["non-interactive", "dry-run"]).multiple(true)))]
pub struct SyncbackCommand {
    /// Path to the project to sync back to.
    #[clap(default_value = "")]
    pub project: PathBuf,

    /// Path to the Roblox file to pull Instances from.
    #[clap(long, short)]
    pub input: PathBuf,

    /// If provided, a list all of the files and directories that will be
    /// added or removed is emitted into stdout.
    #[clap(long, short)]
    pub list: bool,

    /// If provided, syncback will not actually write anything to the file
    /// system. The command will otherwise run normally.
    #[clap(long)]
    pub dry_run: bool,

    /// If provided, the prompt for writing to the file system is skipped.
    #[clap(long, short = 'y')]
    pub non_interactive: bool,

    /// Repeat syncback when the input file changes. Requires -y or --dry-run.
    #[clap(long, requires = "watch_consent")]
    pub watch: bool,
}

impl SyncbackCommand {
    pub fn run(&self, global: GlobalOptions) -> anyhow::Result<()> {
        let path_old = resolve_project_path(&self.project)?;
        let path_new = resolve_path(&self.input)?;

        let input_kind = FileKind::from_path(&path_new).context(UNKNOWN_INPUT_KIND_ERR)?;
        if self.watch {
            ensure!(
                self.non_interactive || self.dry_run,
                "--watch requires -y or --dry-run"
            );
            return self.run_watch(&path_old, &path_new, input_kind, &global);
        }
        self.run_once(&path_old, &path_new, input_kind, &global, None)
    }

    fn run_once(
        &self,
        path_old: &Path,
        path_new: &Path,
        input_kind: FileKind,
        global: &GlobalOptions,
        captured: Option<&[u8]>,
    ) -> anyhow::Result<()> {
        let dom_start_timer = Instant::now();
        let content: Box<dyn io::Read + '_> = match captured {
            Some(bytes) => Box::new(bytes),
            None => Box::new(File::open(path_new)?),
        };
        let dom_new = read_dom(path_new, input_kind, content)?;
        log::debug!(
            "Finished opening file in {:0.02}s",
            dom_start_timer.elapsed().as_secs_f32()
        );

        let vfs = Vfs::new_default()?;
        vfs.set_watch_enabled(false);

        let project_start_timer = Instant::now();
        let session_old = ServeSession::new_for_offline(vfs, path_old)?;
        log::debug!(
            "Finished opening project in {:0.02}s",
            project_start_timer.elapsed().as_secs_f32()
        );

        let mut dom_old = session_old.tree();

        log::debug!("Old root: {}", dom_old.inner().root().class);
        log::debug!("New root: {}", dom_new.root().class);

        if log::log_enabled!(log::Level::Trace) {
            log::trace!("Children of old root:");
            for child in dom_old.inner().root().children() {
                let inst = dom_old.get_instance(*child).unwrap();
                log::trace!("{} (class: {})", inst.name(), inst.class_name());
            }
            log::trace!("Children of new root:");
            for child in dom_new.root().children() {
                let inst = dom_new.get_by_ref(*child).unwrap();
                log::trace!("{} (class: {})", inst.name, inst.class);
            }
        }

        let syncback_timer = Instant::now();
        eprintln!("Beginning syncback...");
        let snapshot = syncback_loop(
            session_old.vfs(),
            &mut dom_old,
            dom_new,
            session_old.root_project(),
        )?;
        log::debug!(
            "Syncback finished in {:.02}s!",
            syncback_timer.elapsed().as_secs_f32()
        );

        let base_path = session_old.root_project().folder_location();
        if self.watch {
            protect_input(&snapshot, base_path, path_new)?;
        }
        if self.list {
            list_files(&snapshot, global.color.into(), base_path)?;
        }

        if !self.dry_run {
            if !self.non_interactive {
                eprintln!(
                    "Would write {} files/folders and remove {} files/folders.",
                    snapshot.added_paths().len(),
                    snapshot.removed_paths().len()
                );
                eprint!("Is this okay? (Y/N): ");
                io::stderr().flush()?;
                let mut line = String::with_capacity(1);
                io::stdin().read_line(&mut line)?;
                line = line.trim().to_lowercase();
                if line != "y" {
                    eprintln!("Aborting due to user input!");
                    return Ok(());
                }
            }
            eprintln!("Writing to the file system...");
            snapshot
                .write_to_vfs(base_path, session_old.vfs())
                .map_err(SyncbackWriteError)?;
            eprintln!("Finished syncback.")
        } else {
            eprintln!(
                "Would write {} files/folders and remove {} files/folders.",
                snapshot.added_paths().len(),
                snapshot.removed_paths().len()
            );
            eprintln!("Aborting before writing to file system due to `--dry-run`");
        }

        // It is potentially prohibitively expensive to drop a ServeSession,
        // and the program is about to exit anyway so we're just going to forget
        // about it.
        drop(dom_old);
        if !self.watch {
            forget(session_old);
        }

        Ok(())
    }

    fn run_watch(
        &self,
        project: &Path,
        input: &Path,
        kind: FileKind,
        global: &GlobalOptions,
    ) -> anyhow::Result<()> {
        let input = dunce::canonicalize(input)
            .with_context(|| format!("Could not watch input {}", input.display()))?;
        let mut watcher =
            StdBackend::new().context("Could not initialize syncback input watcher")?;
        watcher
            .watch(&input)
            .with_context(|| format!("Could not watch input {}", input.display()))?;
        let events = watcher.event_receiver();
        let bytes = fs_err::read(&input)?;
        self.run_once(project, &input, kind, global, Some(&bytes))?;
        let mut last_hash = blake3::hash(&bytes);
        drop(bytes);
        eprintln!("Watching syncback input {}", input.display());
        loop {
            let event = events
                .recv()
                .context("Syncback input watcher disconnected")?;
            if !affects_input(&event, &input) {
                continue;
            }
            let deadline = Instant::now() + Duration::from_millis(100);
            // Do not extend the window for unrelated output notifications or
            // consume events that arrive while planning/writing the next pass.
            while events.recv_deadline(deadline).is_ok() {}
            let _ = watcher.unwatch(&input);
            if let Err(error) = watcher.watch(&input) {
                log::debug!(
                    "Waiting to re-register syncback input {}: {}",
                    input.display(),
                    error
                );
            }
            let result = (|| {
                let bytes = fs_err::read(&input)?;
                let hash = blake3::hash(&bytes);
                if hash != last_hash {
                    self.run_once(project, &input, kind, global, Some(&bytes))?;
                    last_hash = hash;
                }
                Ok::<_, anyhow::Error>(())
            })();
            if let Err(error) = result {
                if error.downcast_ref::<SyncbackWriteError>().is_some() {
                    return Err(error);
                }
                log::warn!(
                    "Syncback input {} was not applied; waiting for a later save: {:#}",
                    input.display(),
                    error
                );
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Could not write syncback output: {0}")]
struct SyncbackWriteError(io::Error);

fn affects_input(event: &VfsEvent, input: &Path) -> bool {
    match event {
        VfsEvent::Create(path) | VfsEvent::Write(path) => path == input,
        VfsEvent::Remove(path) => input.starts_with(path),
        _ => false,
    }
}

fn protect_input(snapshot: &FsSnapshot, base: &Path, input: &Path) -> anyhow::Result<()> {
    for path in snapshot
        .added_files()
        .into_iter()
        .chain(snapshot.removed_paths())
    {
        let path = base.join(path);
        let path = dunce::canonicalize(&path).unwrap_or(path);
        ensure!(
            !input.starts_with(&path),
            "Syncback output would overwrite watched input {} via {}",
            input.display(),
            path.display()
        );
    }
    Ok(())
}

fn read_dom(path: &Path, file_kind: FileKind, content: impl io::Read) -> anyhow::Result<WeakDom> {
    let content = BufReader::new(content);
    match file_kind {
        FileKind::Rbxl => rbx_binary::from_reader(content).with_context(|| {
            format!(
                "Could not deserialize binary place file at {}",
                path.display()
            )
        }),
        FileKind::Rbxlx => rbx_xml::from_reader(content, xml_decode_config())
            .with_context(|| format!("Could not deserialize XML place file at {}", path.display())),
        FileKind::Rbxm => {
            let temp_tree = rbx_binary::from_reader(content).with_context(|| {
                format!(
                    "Could not deserialize binary place file at {}",
                    path.display()
                )
            })?;

            process_model_dom(temp_tree)
        }
        FileKind::Rbxmx => {
            let temp_tree =
                rbx_xml::from_reader(content, xml_decode_config()).with_context(|| {
                    format!("Could not deserialize XML model file at {}", path.display())
                })?;
            process_model_dom(temp_tree)
        }
    }
}

fn process_model_dom(dom: WeakDom) -> anyhow::Result<WeakDom> {
    let temp_children = dom.root().children();
    if temp_children.len() == 1 {
        let real_root = dom.get_by_ref(temp_children[0]).unwrap();
        let mut new_tree = WeakDom::new(InstanceBuilder::new(real_root.class));
        for (name, property) in &real_root.properties {
            new_tree
                .root_mut()
                .properties
                .insert(*name, property.to_owned());
        }

        let children = dom.clone_multiple_into_external(real_root.children(), &mut new_tree);
        for child in children {
            new_tree.transfer_within(child, new_tree.root_ref());
        }
        Ok(new_tree)
    } else {
        anyhow::bail!(
            "Rojo does not currently support models with more \
        than one Instance at the Root!"
        );
    }
}

fn xml_decode_config() -> rbx_xml::DecodeOptions<'static> {
    rbx_xml::DecodeOptions::new().property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown)
}

/// The different kinds of input that Rojo can syncback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    /// An XML model file.
    Rbxmx,

    /// An XML place file.
    Rbxlx,

    /// A binary model file.
    Rbxm,

    /// A binary place file.
    Rbxl,
}

impl FileKind {
    fn from_path(output: &Path) -> Option<FileKind> {
        let extension = output.extension()?.to_str()?;

        match extension {
            "rbxlx" => Some(FileKind::Rbxlx),
            "rbxmx" => Some(FileKind::Rbxmx),
            "rbxl" => Some(FileKind::Rbxl),
            "rbxm" => Some(FileKind::Rbxm),
            _ => None,
        }
    }
}

fn list_files(snapshot: &FsSnapshot, color: ColorChoice, base_path: &Path) -> io::Result<()> {
    let no_color = ColorSpec::new();
    let mut add_color = ColorSpec::new();
    add_color.set_fg(Some(Color::Green));
    let mut remove_color = ColorSpec::new();
    remove_color.set_fg(Some(Color::Red));

    let writer = BufferWriter::stdout(color);
    let mut buffer = writer.buffer();

    let added = snapshot.added_paths();
    if !added.is_empty() {
        buffer.set_color(&add_color)?;
        for path in added {
            writeln!(
                &mut buffer,
                "Writing {}",
                display_absolute(path.strip_prefix(base_path).unwrap_or(path))
            )?;
        }
    }
    let removed = snapshot.removed_paths();
    if !removed.is_empty() {
        buffer.set_color(&remove_color)?;
        for path in removed {
            writeln!(
                &mut buffer,
                "Removing {}",
                display_absolute(path.strip_prefix(base_path).unwrap_or(path))
            )?;
        }
    }
    buffer.set_color(&no_color)?;

    writer.print(&buffer)
}

#[cfg(test)]
mod watch_tests {
    use super::*;

    #[test]
    fn snapshot_guard_protects_input_and_removal_ancestors_but_allows_other_output() {
        let dir = tempfile::tempdir().unwrap();
        let base = dunce::canonicalize(dir.path()).unwrap();
        fs_err::create_dir(base.join("src")).unwrap();
        let input = base.join("src/input.rbxm");
        fs_err::write(&input, b"input").unwrap();
        let overwrite = FsSnapshot::new().with_added_file("src/input.rbxm", Vec::new());
        assert!(protect_input(&overwrite, &base, &input).is_err());
        let alias = FsSnapshot::new().with_added_file("src/../src/input.rbxm", Vec::new());
        assert!(protect_input(&alias, &base, &input).is_err());
        let mut remove_file = FsSnapshot::new();
        remove_file.remove_file("src/input.rbxm");
        assert!(protect_input(&remove_file, &base, &input).is_err());
        let mut remove_dir = FsSnapshot::new();
        remove_dir.remove_dir("src");
        assert!(protect_input(&remove_dir, &base, &input).is_err());
        let other = FsSnapshot::new()
            .with_added_dir("src")
            .with_added_file("src/Example.luau", Vec::new());
        assert!(protect_input(&other, &base, &input).is_ok());
    }
}
