use std::{
    fs,
    io::Write,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, ensure, Context};
use clap::Parser;
use memofs::Vfs;
use rbx_dom_weak::{types::Ref, InstanceBuilder, WeakDom};
use roblox_install::RobloxStudio;

use crate::{serve_session::ServeSession, web::LiveServer};

use super::{resolve_path, resolve_project_path, GlobalOptions};

const MARKER: &str = "__RojoOpenLaunch_v1";

/// Build or copy a place, serve its project and open it in local Windows Studio.
#[derive(Debug, Parser)]
pub struct OpenCommand {
    /// Project path/stem or existing .rbxl/.rbxlx place. Defaults to this directory.
    #[clap(default_value = "")]
    pub target: PathBuf,
    /// Project for an existing place target. Defaults to this directory.
    #[clap(long)]
    pub project: Option<PathBuf>,
    /// New place output; existing files are never overwritten. Default: a temporary copy.
    #[clap(long, short)]
    pub output: Option<PathBuf>,
    /// Loopback serve port. Defaults to the project's port or 34872; must be positive.
    #[clap(long)]
    pub port: Option<u16>,
    /// Use an already installed compatible plugin without installing the embedded copy.
    #[clap(long)]
    pub skip_plugin_install: bool,
}

impl OpenCommand {
    pub fn run(self, _global: GlobalOptions) -> anyhow::Result<()> {
        ensure!(
            cfg!(windows),
            "rojo open currently supports local Windows Studio only"
        );
        let existing = matches!(
            self.target.extension().and_then(|value| value.to_str()),
            Some("rbxl" | "rbxlx")
        );
        ensure!(
            existing || self.project.is_none(),
            "--project requires an existing place target"
        );
        ensure!(self.port != Some(0), "--port must be positive");
        let project_path = if existing {
            self.project.as_deref().unwrap_or_else(|| Path::new(""))
        } else {
            &self.target
        };
        let vfs = Vfs::new_default()?;
        let session = Arc::new(ServeSession::new(vfs, resolve_project_path(project_path)?)?);
        let port = self
            .port
            .or_else(|| session.project_port())
            .unwrap_or(34872);
        ensure!(port > 0, "The open workflow requires a positive serve port");
        let mut dom = if existing {
            read_place(&resolve_path(&self.target)?)?
        } else {
            let tree = session.tree();
            ensure!(
                tree.root().class_name() == "DataModel",
                "rojo open requires a DataModel project root when building a place"
            );
            let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
            let children = tree
                .inner()
                .clone_multiple_into_external(tree.inner().root().children(), &mut dom);
            for child in children {
                dom.transfer_within(child, dom.root_ref());
            }
            dom
        };
        add_marker(&mut dom, &session, port)?;
        let output = PlaceOutput::new(self.output.as_deref())?;
        let bytes = place_bytes(&dom, &output.path)?;
        let studio = RobloxStudio::locate().context("Could not locate Studio for rojo open")?;
        ensure!(
            studio.application_path().is_file(),
            "Studio application does not exist: {}",
            studio.application_path().display()
        );
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let hosts = session.serve_allowed_hosts().to_vec();
        LiveServer::new(session).start_after_bind(address, hosts, || {
            if self.skip_plugin_install {
                println!("Using the installed plugin; it must support rojo open launch markers.");
            } else {
                super::plugin::ensure_embedded_plugin(&studio)?;
            }
            publish_new_file(&output.path, &bytes)?;
            output.capture(&bytes);
            // The pinned, repeatedly verified native runner launches Studio with
            // the place path as one argument. Keep paths out of a shell.
            Command::new(studio.application_path())
                .arg(&output.path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .with_context(|| {
                    format!("Could not open Studio place {}", output.path.display())
                })?;
            println!("Opened place copy: {}", output.path.display());
            if output.directory.is_some() {
                println!("Use Save As in Studio to keep an edited place permanently.");
            }
            println!(
                "Rojo is serving at http://127.0.0.1:{port}; accept the initial sync in Studio."
            );
            Ok(())
        })
    }
}

fn read_place(path: &Path) -> anyhow::Result<WeakDom> {
    let bytes = fs::read(path)
        .with_context(|| format!("Could not read existing place {}", path.display()))?;
    match path.extension().and_then(|value| value.to_str()) {
        Some("rbxl") => Ok(rbx_binary::from_reader(bytes.as_slice())?),
        Some("rbxlx") => Ok(rbx_xml::from_reader(
            bytes.as_slice(),
            rbx_xml::DecodeOptions::new()
                .property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown),
        )?),
        _ => bail!("Expected .rbxl or .rbxlx place: {}", path.display()),
    }
}

fn add_marker(dom: &mut WeakDom, session: &ServeSession, port: u16) -> anyhow::Result<()> {
    let mut storage = dom
        .root()
        .children()
        .iter()
        .copied()
        .filter(|id| dom.get_by_ref(*id).unwrap().class == "ReplicatedStorage");
    let first = storage.next();
    ensure!(
        storage.next().is_none(),
        "Place contains ambiguous ReplicatedStorage services"
    );
    let storage = first
        .unwrap_or_else(|| dom.insert(dom.root_ref(), InstanceBuilder::new("ReplicatedStorage")));
    ensure!(
        !dom.get_by_ref(storage)
            .unwrap()
            .children()
            .iter()
            .any(|id| dom.get_by_ref(*id).unwrap().name == MARKER),
        "Reserved launch marker already exists at ReplicatedStorage.{MARKER}"
    );
    let created = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    ensure!(
        session.project_name().len() <= 256
            && !session.project_name().chars().any(char::is_control),
        "Open project name must be at most 256 UTF-8 bytes without control characters"
    );
    let details = serde_json::to_string(&serde_json::json!({
        "version": 1, "host": "127.0.0.1", "port": port,
        "sessionId": session.session_id().to_string(), "projectName": session.project_name(),
        "createdAt": created, "expiresAt": created + 600
    }))?;
    ensure!(
        details.len() <= 4096,
        "Open launch details exceed 4096 bytes"
    );
    dom.insert(
        storage,
        InstanceBuilder::new("Folder").with_name(MARKER).with_child(
            InstanceBuilder::new("StringValue")
                .with_name("Details")
                .with_property("Value", details),
        ),
    );
    Ok(())
}

fn place_bytes(dom: &WeakDom, path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    match path.extension().and_then(|value| value.to_str()) {
        Some("rbxl") => {
            crate::binary_serialization::to_writer(&mut bytes, dom, dom.root().children())?
        }
        Some("rbxlx") => rbx_xml::to_writer(
            &mut bytes,
            dom,
            dom.root().children(),
            rbx_xml::EncodeOptions::new()
                .property_behavior(rbx_xml::EncodePropertyBehavior::WriteUnknown),
        )?,
        _ => bail!("Open output must be .rbxl or .rbxlx: {}", path.display()),
    }
    Ok(bytes)
}

struct RemoveFile(PathBuf);
impl Drop for RemoveFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Publish a completed sibling file without ever replacing an existing entry.
pub(super) fn publish_new_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("Output needs a parent directory")?;
    let temporary = parent.join(format!(".rojo-open-{}.tmp", Ref::new()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| {
            format!(
                "Could not create temporary output beside {}",
                path.display()
            )
        })?;
    let temporary = RemoveFile(temporary);
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    // A hard link publishes the completed file atomically and fails if another
    // writer won the name. Both paths are siblings on the same filesystem.
    fs::hard_link(&temporary.0, path).with_context(|| {
        format!(
            "Could not publish new file without overwriting: {}",
            path.display()
        )
    })?;
    Ok(())
}

struct PlaceOutput {
    path: PathBuf,
    directory: Option<PathBuf>,
    original: std::cell::Cell<Option<blake3::Hash>>,
}

impl PlaceOutput {
    fn new(output: Option<&Path>) -> anyhow::Result<Self> {
        let (path, directory) = if let Some(output) = output {
            let path = resolve_path(output)?.into_owned();
            ensure!(
                fs::symlink_metadata(&path)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
                "Open output already exists or cannot be inspected: {}",
                path.display()
            );
            #[cfg(windows)]
            super::build::validate_output_path(&path)?;
            (path, None)
        } else {
            let directory = std::env::temp_dir().join(format!("rojo-open-{}", Ref::new()));
            fs::create_dir(&directory)?;
            (directory.join("Place.rbxl"), Some(directory))
        };
        Ok(Self {
            path,
            directory,
            original: std::cell::Cell::new(None),
        })
    }
    fn capture(&self, bytes: &[u8]) {
        self.original.set(Some(blake3::hash(bytes)));
    }
}

impl Drop for PlaceOutput {
    fn drop(&mut self) {
        if let Some(directory) = &self.directory {
            if let Some(original) = self.original.get() {
                match fs::read(&self.path) {
                    Ok(bytes) if blake3::hash(&bytes) == original => {
                        let _ = fs::remove_file(&self.path);
                    }
                    Ok(_) => {
                        log::warn!("Preserving edited temporary place: {}", self.path.display())
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => log::warn!(
                        "Preserving temporary place {}: {error}",
                        self.path.display()
                    ),
                }
            }
            let _ = fs::remove_dir(directory);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_never_overwrites_an_existing_entry_and_cleans_owned_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Place.rbxl");
        publish_new_file(&path, b"first").unwrap();
        assert!(publish_new_file(&path, b"second").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"first");
        let folder = dir.path().join("Folder.rbxl");
        fs::create_dir(&folder).unwrap();
        assert!(publish_new_file(&folder, b"second").is_err());
        assert!(folder.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn temporary_place_cleanup_preserves_studio_saved_changes() {
        let output = PlaceOutput::new(None).unwrap();
        let path = output.path.clone();
        let directory = output.directory.clone().unwrap();
        publish_new_file(&path, b"generated").unwrap();
        output.capture(b"generated");
        drop(output);
        assert!(!directory.exists());
        let output = PlaceOutput::new(None).unwrap();
        let path = output.path.clone();
        let directory = output.directory.clone().unwrap();
        publish_new_file(&path, b"generated").unwrap();
        output.capture(b"generated");
        fs::write(&path, b"saved by Studio").unwrap();
        drop(output);
        assert_eq!(fs::read(&path).unwrap(), b"saved by Studio");
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
