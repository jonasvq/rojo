use std::{
    fs::{self, File},
    io::BufWriter,
    path::Path,
};

use clap::Parser;
use memofs::{InMemoryFs, Vfs, VfsSnapshot};
use roblox_install::RobloxStudio;

use crate::serve_session::ServeSession;

static PLUGIN_BINCODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/plugin.bincode"));
static PLUGIN_FILE_NAME: &str = "RojoManagedPlugin.rbxm";

/// Install Rojo's plugin.
#[derive(Debug, Parser)]
pub struct PluginCommand {
    #[clap(subcommand)]
    subcommand: PluginSubcommand,
}

/// Manages Rojo's Roblox Studio plugin.
#[derive(Debug, Parser)]
pub enum PluginSubcommand {
    /// Install the plugin in Roblox Studio's plugins folder. If the plugin is
    /// already installed, installing it again will overwrite the current plugin
    /// file.
    Install,

    /// Removes the plugin if it is installed.
    Uninstall,
}

impl PluginCommand {
    pub fn run(self) -> anyhow::Result<()> {
        self.subcommand.run()
    }
}

impl PluginSubcommand {
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            PluginSubcommand::Install => install_plugin(),
            PluginSubcommand::Uninstall => uninstall_plugin(),
        }
    }
}

fn initialize_plugin() -> anyhow::Result<ServeSession> {
    let plugin_snapshot: VfsSnapshot = bincode::deserialize(PLUGIN_BINCODE)
        .expect("Rojo's plugin was not properly packed into Rojo's binary");

    let mut in_memory_fs = InMemoryFs::new();
    in_memory_fs.load_snapshot("/plugin", plugin_snapshot)?;

    let vfs = Vfs::new(in_memory_fs);
    Ok(ServeSession::new(vfs, "/plugin")?)
}

fn install_plugin() -> anyhow::Result<()> {
    let studio = RobloxStudio::locate()?;

    let plugins_folder_path = studio.plugins_path();

    if !plugins_folder_path.exists() {
        log::debug!("Creating Roblox Studio plugins folder");
        fs::create_dir(plugins_folder_path)?;
    }

    let plugin_path = plugins_folder_path.join(PLUGIN_FILE_NAME);
    log::debug!("Writing plugin to {}", plugin_path.display());

    let mut file = BufWriter::new(File::create(plugin_path)?);

    let session = initialize_plugin()?;
    let tree = session.tree();
    let root_id = tree.get_root_id();

    crate::binary_serialization::to_writer(&mut file, tree.inner(), &[root_id])?;

    Ok(())
}

pub(super) fn ensure_embedded_plugin(studio: &RobloxStudio) -> anyhow::Result<()> {
    let bytes = embedded_plugin_bytes()?;
    let path = studio.plugins_path().join(PLUGIN_FILE_NAME);
    fs::create_dir_all(studio.plugins_path())?;
    ensure_plugin_at(&path, &bytes)
}

fn embedded_plugin_bytes() -> anyhow::Result<Vec<u8>> {
    let session = initialize_plugin()?;
    let mut bytes = Vec::new();
    {
        let tree = session.tree();
        crate::binary_serialization::to_writer(&mut bytes, tree.inner(), &[tree.get_root_id()])?;
    }
    Ok(bytes)
}

fn ensure_plugin_at(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => verify_existing_plugin(path, bytes, metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match super::open::publish_new_file(path, bytes) {
                Ok(()) => Ok(()),
                // A concurrent installer may have won the exclusive name.
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
                {
                    verify_existing_plugin(path, bytes, fs::symlink_metadata(path)?)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn verify_existing_plugin(path: &Path, bytes: &[u8], metadata: fs::Metadata) -> anyhow::Result<()> {
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Managed plugin destination is not a regular file: {}",
        path.display()
    );
    anyhow::ensure!(fs::read(path)? == bytes,
        "A different managed Rojo plugin exists at {}. Run rojo plugin install explicitly, or use --skip-plugin-install with a compatible installed plugin.",
        path.display());
    Ok(())
}

fn uninstall_plugin() -> anyhow::Result<()> {
    let studio = RobloxStudio::locate()?;

    let plugin_path = studio.plugins_path().join(PLUGIN_FILE_NAME);

    if plugin_path.exists() {
        log::debug!("Removing existing plugin from {}", plugin_path.display());
        fs::remove_file(plugin_path)?;
    } else {
        log::debug!("Plugin not installed at {}", plugin_path.display());
    }

    Ok(())
}

#[test]
fn plugin_initialize() {
    let _ = initialize_plugin().unwrap();
}

#[cfg(test)]
mod open_install_tests {
    use super::*;

    #[test]
    fn actual_embedded_plugin_bytes_are_repeatable_and_include_launch_support() {
        let one = embedded_plugin_bytes().unwrap();
        let two = embedded_plugin_bytes().unwrap();
        assert_eq!(one, two);
        let dom = rbx_binary::from_reader(one.as_slice()).unwrap();
        assert!(dom
            .descendants()
            .any(|instance| instance.name == "OpenLaunch" && instance.class == "ModuleScript"));
        assert!(!dom
            .descendants()
            .any(|instance| instance.name == "OpenLaunch.spec"));
    }

    #[test]
    fn guarded_install_creates_once_reuses_identical_and_refuses_other_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Managed.rbxm");
        ensure_plugin_at(&path, b"current embedded plugin").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        ensure_plugin_at(&path, b"current embedded plugin").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let error = ensure_plugin_at(&path, b"different version")
            .unwrap_err()
            .to_string();
        assert!(error.contains("rojo plugin install"));
        assert_eq!(fs::read(&path).unwrap(), b"current embedded plugin");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        let folder = dir.path().join("Folder.rbxm");
        fs::create_dir(&folder).unwrap();
        assert!(ensure_plugin_at(&folder, b"plugin").is_err());
        assert!(folder.is_dir());
    }

    #[test]
    fn concurrent_new_file_publishing_does_not_replace_the_winner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Managed.rbxm");
        std::thread::scope(|scope| {
            let one = scope.spawn(|| ensure_plugin_at(&path, b"same version"));
            let two = scope.spawn(|| ensure_plugin_at(&path, b"same version"));
            one.join().unwrap().unwrap();
            two.join().unwrap().unwrap();
        });
        assert_eq!(fs::read(&path).unwrap(), b"same version");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn guarded_install_refuses_a_symlink_even_with_identical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("UserPlugin.rbxm");
        let link = dir.path().join("Managed.rbxm");
        fs::write(&real, b"same version").unwrap();
        if let Err(error) = std::os::windows::fs::symlink_file(&real, &link) {
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            return;
        }
        assert!(ensure_plugin_at(&link, b"same version").is_err());
        assert_eq!(fs::read(&real).unwrap(), b"same version");
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
}
