mod cloud;

use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::Parser;
use memofs::Vfs;

use super::resolve_project_path;

/// Builds the project and uploads it to Roblox.
#[derive(Debug, Parser)]
pub struct UploadCommand {
    /// Path to the project to upload. Defaults to the current directory.
    #[clap(default_value = "")]
    pub project: PathBuf,

    /// Deprecated and ignored. Use an Open Cloud API key.
    #[clap(long)]
    pub cookie: Option<String>,

    /// Open Cloud API key for updating an existing model or publishing a place.
    #[clap(long = "api_key")]
    pub api_key: Option<String>,

    /// Universe ID for place publishing. Must be omitted for model updates.
    #[clap(long = "universe_id")]
    pub universe_id: Option<u64>,

    /// Asset ID to upload to.
    #[clap(long = "asset_id")]
    pub asset_id: u64,
}

impl UploadCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let project_path = resolve_project_path(&self.project)?;
        let vfs = Vfs::new_default()?;
        vfs.set_watch_enabled(false);
        let project = crate::Project::load_initial_project(&vfs, &project_path)?;
        let tree = crate::build_project(&vfs, &project)?;
        let root = tree.inner().root();
        if self.cookie.is_some() {
            log::warn!(
                "--cookie is deprecated and ignored. Uploads require an Open Cloud --api_key."
            );
        }
        let target = self.target(root.class == "DataModel")?;
        let client = cloud::Cloud::new(self.api_key.as_deref().unwrap())?;
        let bytes = encode_tree(&tree)?;
        client.upload(bytes, target)
    }

    fn target(&self, place: bool) -> anyhow::Result<cloud::Target> {
        if self.api_key.is_none() {
            if place {
                bail!("Place uploads require Open Cloud. Pass --api_key and --universe_id; legacy cookies are no longer supported.");
            }
            bail!("Model uploads require Open Cloud. Pass --api_key; legacy cookies are no longer supported.");
        }
        anyhow::ensure!(
            !self.api_key.as_deref().unwrap().trim().is_empty(),
            "--api_key must not be empty"
        );
        anyhow::ensure!(self.asset_id > 0, "--asset_id must be positive");
        if place {
            let universe_id = self
                .universe_id
                .context("--universe_id must be provided for place uploads")?;
            anyhow::ensure!(universe_id > 0, "--universe_id must be positive");
            Ok(cloud::Target::Place {
                universe_id,
                asset_id: self.asset_id,
            })
        } else {
            anyhow::ensure!(
                self.universe_id.is_none(),
                "--universe_id is only valid for place uploads, not models"
            );
            Ok(cloud::Target::Model {
                asset_id: self.asset_id,
            })
        }
    }
}

fn encode_tree(tree: &crate::RojoTree) -> anyhow::Result<Vec<u8>> {
    let root = tree.inner().root();
    let root_id = [root.referent()];
    let ids = if root.class == "DataModel" {
        root.children()
    } else {
        &root_id
    };
    let mut bytes = Vec::new();
    crate::binary_serialization::to_writer(&mut bytes, tree.inner(), ids)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InstanceSnapshot, RojoTree};
    use rbx_dom_weak::types::Variant;

    fn command(key: Option<&str>, universe: Option<u64>, asset_id: u64) -> UploadCommand {
        UploadCommand {
            project: PathBuf::new(),
            cookie: Some("IGNORED_TEST_COOKIE".into()),
            api_key: key.map(str::to_owned),
            universe_id: universe,
            asset_id,
        }
    }

    #[test]
    fn upload_mode_and_identifiers_are_validated_before_transport() {
        assert!(matches!(
            command(Some("key"), None, 42).target(false).unwrap(),
            cloud::Target::Model { asset_id: 42 }
        ));
        assert!(matches!(
            command(Some("key"), Some(10), 42).target(true).unwrap(),
            cloud::Target::Place {
                universe_id: 10,
                asset_id: 42
            }
        ));
        for (place, key, universe, id) in [
            (false, None, None, 42),
            (true, None, Some(10), 42),
            (false, Some(" "), None, 42),
            (true, Some("key"), None, 42),
            (true, Some("key"), Some(0), 42),
            (false, Some("key"), Some(10), 42),
            (false, Some("key"), None, 0),
        ] {
            assert!(command(key, universe, id).target(place).is_err());
        }
    }

    #[test]
    fn model_payload_includes_root_but_place_payload_contains_only_children() {
        for class in ["Folder", "DataModel"] {
            let tree = RojoTree::new(
                InstanceSnapshot::new()
                    .name("ProjectRoot")
                    .class_name(class)
                    .children(vec![InstanceSnapshot::new()
                        .name("Child")
                        .class_name("StringValue")
                        .property("Value", "data")]),
            );
            let bytes = encode_tree(&tree).unwrap();
            let dom = rbx_binary::from_reader(bytes.as_slice()).unwrap();
            let roots = dom.root().children();
            assert_eq!(roots.len(), 1);
            let root = dom.get_by_ref(roots[0]).unwrap();
            assert_eq!(
                root.name,
                if class == "DataModel" {
                    "Child"
                } else {
                    "ProjectRoot"
                }
            );
            let child = dom
                .descendants()
                .find(|instance| instance.name == "Child")
                .unwrap();
            assert_eq!(
                child.properties.get(&"Value".into()),
                Some(&Variant::String("data".into()))
            );
        }
    }

    #[test]
    fn upload_payload_uses_legacy_mesh_volume_fidelity_defaults() {
        let tree = RojoTree::new(
            InstanceSnapshot::new()
                .name("Root")
                .class_name("Folder")
                .children(vec![
                    InstanceSnapshot::new()
                        .name("Legacy")
                        .class_name("MeshPart"),
                    InstanceSnapshot::new()
                        .name("Modern")
                        .class_name("MeshPart")
                        .property("UnscaledVolume", 12.0_f32),
                ]),
        );
        let bytes = encode_tree(&tree).unwrap();
        let dom = rbx_binary::from_reader(bytes.as_slice()).unwrap();
        for (name, value) in [("Legacy", 0.0_f32), ("Modern", 12.0_f32)] {
            let instance = dom
                .descendants()
                .find(|instance| instance.name == name)
                .unwrap();
            assert_eq!(
                instance.properties.get(&"UnscaledVolume".into()),
                Some(&Variant::Float32(value))
            );
        }
    }
}
