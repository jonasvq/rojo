use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context};
use memofs::Vfs;
use rbx_dom_weak::{
    types::{Ref, Variant},
    InstanceBuilder, WeakDom,
};

use super::{BuildCommand, OutputKind};
use crate::cli::{resolve_path, resolve_project_path};
use crate::{Project, RojoTree};

pub(super) fn run(command: &BuildCommand, input: &Path) -> anyhow::Result<()> {
    ensure!(
        !command.watch && command.plugin.is_none(),
        "--input conflicts with --watch and --plugin"
    );
    let output = command
        .output
        .as_deref()
        .context("--input requires --output")?;
    #[cfg(windows)]
    super::validate_output_path(output)?;
    let input_kind = place_kind(input)?;
    let output_kind = place_kind(output)?;
    super::validate_compression(command.compress, output_kind)?;
    let compression = command.compress.map(Into::into).unwrap_or_default();
    let input = resolve_path(input)?;
    let output = resolve_path(output)?;
    let captured = fs_err::read(&input)
        .with_context(|| format!("Could not read input place {}", input.display()))?;
    let mut dom = match input_kind {
        OutputKind::Rbxl => rbx_binary::from_reader(captured.as_slice())?,
        OutputKind::Rbxlx => rbx_xml::from_reader(
            captured.as_slice(),
            rbx_xml::DecodeOptions::new()
                .property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown),
        )?,
        _ => unreachable!(),
    };
    let vfs = Vfs::new_default()?;
    vfs.set_watch_enabled(false);
    let (project, snapshot) = if super::stdin::is_stdin(&command.project) {
        let project = super::stdin::read_project(&vfs)?;
        let snapshot = crate::builder::stdin_snapshot(&vfs, &project)
            .context("Could not build stdin project")?;
        (project, snapshot)
    } else {
        let project =
            Project::load_initial_project(&vfs, &resolve_project_path(&command.project)?)?;
        let snapshot = crate::builder::project_snapshot(&vfs, &project)?;
        (project, snapshot)
    };
    // Track authored migration flags through the shared builder without adding
    // public metadata. Both traversals preserve breadth-first child order.
    let mut queue = VecDeque::from([&snapshot]);
    let mut migration_authored = Vec::new();
    while let Some(instance) = queue.pop_front() {
        migration_authored.push(
            instance
                .properties
                .contains_key(&"NeedsPivotMigration".into()),
        );
        queue.extend(&instance.children);
    }
    let source = crate::builder::build_snapshot(snapshot);
    let authored: HashSet<_> = source
        .inner()
        .descendants()
        .zip(migration_authored)
        .filter_map(|(instance, authored)| authored.then_some(instance.referent()))
        .collect();
    ensure!(
        source.inner().root().class == "DataModel",
        "--input requires a DataModel project root"
    );
    overlay(&mut dom, &source, &authored)?;
    write_output(&output, &dom, output_kind, compression, || {
        ensure!(
            fs_err::read(&input)? == captured,
            "Input place changed before output replacement: {}",
            input.display()
        );
        Ok(())
    })
    .with_context(|| format!("Could not update output place {}", output.display()))?;
    println!(
        "Updated place {} from project {}",
        output.display(),
        project.file_location.display()
    );
    Ok(())
}

fn place_kind(path: &Path) -> anyhow::Result<OutputKind> {
    let kind = OutputKind::from_output_path(path);
    ensure!(
        matches!(kind, Some(OutputKind::Rbxl | OutputKind::Rbxlx)),
        "Existing-place builds require .rbxl or .rbxlx: {}",
        path.display()
    );
    Ok(kind.unwrap())
}

struct Node {
    source: Ref,
    target: Ref,
    parent: Ref,
    new: bool,
}

fn overlay(
    dom: &mut WeakDom,
    source: &RojoTree,
    authored_migration: &HashSet<Ref>,
) -> anyhow::Result<()> {
    let mut bindings = HashMap::from([(source.get_root_id(), dom.root_ref())]);
    let mut nodes = vec![Node {
        source: source.get_root_id(),
        target: dom.root_ref(),
        parent: Ref::none(),
        new: false,
    }];
    let mut queue = VecDeque::from([0]);
    let mut removed = Vec::new();
    while let Some(index) = queue.pop_front() {
        let node = &nodes[index];
        let parent = node.target;
        let desired = source.get_instance(node.source).unwrap();
        let existing = dom
            .get_by_ref(parent)
            .map(|instance| instance.children())
            .unwrap_or_default();
        let mut by_name_class = HashMap::new();
        let mut existing_names = HashSet::new();
        for id in existing {
            let instance = dom.get_by_ref(*id).unwrap();
            existing_names.insert(instance.name.as_str());
            by_name_class
                .entry((instance.name.as_str(), instance.class))
                .or_insert_with(Vec::new)
                .push(*id);
        }
        let mut used = HashSet::new();
        for child_id in desired.children() {
            let child = source.get_instance(*child_id).unwrap();
            let matching = by_name_class
                .get(&(child.name(), child.class_name()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let path = source.inner().full_path_of(*child_id, ".");
            ensure!(
                matching.len() <= 1,
                "Ambiguous existing place children at {path} ({})",
                child.class_name()
            );
            ensure!(
                !matching.is_empty() || !existing_names.contains(child.name()),
                "Existing place class mismatch at {path}: expected {}",
                child.class_name()
            );
            let target = matching.first().copied().unwrap_or_else(Ref::new);
            ensure!(used.insert(target), "Ambiguous project children at {path}");
            bindings.insert(*child_id, target);
            nodes.push(Node {
                source: *child_id,
                target,
                parent,
                new: matching.is_empty(),
            });
            queue.push_back(nodes.len() - 1);
        }
        if !desired.metadata().ignore_unknown_instances {
            removed.extend(existing.iter().filter(|id| !used.contains(id)).copied());
        }
    }

    // Resolve every project reference before changing the output DOM. Existing
    // input references use a separate ID space and must never be remapped.
    let mut properties = Vec::with_capacity(nodes.len());
    for node in &nodes {
        let desired = source.get_instance(node.source).unwrap();
        let mut values = desired.properties().clone();
        // RojoTree adds this default for fresh builds; it is not an authored
        // instruction to migrate an existing model's pivot state.
        if !node.new && !authored_migration.contains(&node.source) {
            values.remove(&"NeedsPivotMigration".into());
        }
        // An unresolved Rojo_Target directive builds as the property's default
        // nil. In an overlay it must also replace an existing non-nil value,
        // rather than being mistaken for an unspecified property.
        if let Some(Variant::Attributes(attributes)) =
            desired.properties().get(&"Attributes".into())
        {
            for (name, value) in attributes.iter() {
                let valid = matches!(value, Variant::String(_))
                    || matches!(value, Variant::BinaryString(bytes) if std::str::from_utf8(bytes.as_ref()).is_ok());
                if let Some(property) = name
                    .strip_prefix(crate::REF_POINTER_ATTRIBUTE_PREFIX)
                    .filter(|_| valid)
                {
                    values
                        .entry(property.into())
                        .or_insert(Variant::Ref(Ref::none()));
                }
            }
        }
        for (name, value) in &mut values {
            if let Variant::Ref(target) = value {
                if target.is_some() {
                    *target = *bindings.get(target).with_context(|| {
                        format!(
                            "Unresolved project reference at {}.{name}",
                            source.inner().full_path_of(node.source, ".")
                        )
                    })?;
                }
            }
        }
        properties.push(values);
    }
    for id in &removed {
        log::warn!(
            "Removing unknown existing place object {}",
            dom.full_path_of(*id, ".")
        );
    }
    if !removed.is_empty() {
        log::warn!(
            "Removing {} unknown object roots according to project ignore rules",
            removed.len()
        );
    }
    let deleted: HashSet<_> = removed
        .iter()
        .flat_map(|id| dom.descendants_of(*id).map(|instance| instance.referent()))
        .collect();
    dom.destroy_many(&removed);
    for (node, properties) in nodes.iter().zip(properties) {
        let desired = source.get_instance(node.source).unwrap();
        if node.new {
            dom.insert(
                node.parent,
                InstanceBuilder::new(desired.class_name())
                    .with_name(desired.name())
                    .with_referent(node.target)
                    .with_properties(properties),
            );
        } else {
            dom.get_by_ref_mut(node.target)
                .unwrap()
                .properties
                .extend(properties);
        }
    }
    let ids: Vec<_> = dom
        .descendants()
        .map(|instance| instance.referent())
        .collect();
    for id in ids {
        for value in dom.get_by_ref_mut(id).unwrap().properties.values_mut() {
            if let Variant::Ref(target) = value {
                if deleted.contains(target) {
                    *target = Ref::none();
                }
            }
        }
    }
    Ok(())
}

struct TemporaryOutput(PathBuf);
impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs_err::remove_file(&self.0);
    }
}

fn write_output(
    output: &Path,
    dom: &WeakDom,
    kind: OutputKind,
    compression: rbx_binary::CompressionType,
    before_replace: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let parent = output.parent().context("Output needs a parent directory")?;
    let permissions = match fs_err::metadata(output) {
        Ok(metadata) => {
            ensure!(
                !metadata.permissions().readonly(),
                "Output is read-only: {}",
                output.display()
            );
            Some(metadata.permissions())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let temporary = parent.join(format!(".rojo-{}.tmp", Ref::new()));
    let file = fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .with_context(|| {
            format!(
                "Could not create temporary output beside {}",
                output.display()
            )
        })?;
    // Only install the cleanup guard after create_new succeeds: never delete
    // another invocation's file if a name collision occurs.
    let temporary = TemporaryOutput(temporary);
    let mut writer = BufWriter::new(file);
    match kind {
        OutputKind::Rbxl => crate::binary_serialization::to_writer_with_compression(
            &mut writer,
            dom,
            dom.root().children(),
            compression,
        )?,
        OutputKind::Rbxlx => rbx_xml::to_writer(
            &mut writer,
            dom,
            dom.root().children(),
            rbx_xml::EncodeOptions::new()
                .property_behavior(rbx_xml::EncodePropertyBehavior::WriteUnknown),
        )?,
        _ => unreachable!(),
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    if let Some(permissions) = permissions {
        writer.get_ref().set_permissions(permissions)?;
    }
    drop(writer);
    before_replace()?;
    fs_err::rename(&temporary.0, output)
        .with_context(|| format!("Could not replace output place {}", output.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dom(valid: bool) -> WeakDom {
        WeakDom::new(InstanceBuilder::new("DataModel").with_child(
            InstanceBuilder::new("Part").with_property(
                "Archivable",
                if valid {
                    Variant::Bool(true)
                } else {
                    Variant::String("invalid boolean".into())
                },
            ),
        ))
    }

    fn no_temps(parent: &Path) {
        assert!(!fs_err::read_dir(parent).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".rojo-")));
    }

    #[test]
    fn serialization_and_precommit_failures_keep_old_output_and_clean_only_owned_temp() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output.rbxl");
        let unrelated = dir.path().join(".rojo-unrelated.tmp");
        fs_err::write(&unrelated, b"preserve other invocation").unwrap();
        fs_err::write(&output, b"original").unwrap();
        assert!(write_output(
            &output,
            &dom(false),
            OutputKind::Rbxl,
            rbx_binary::CompressionType::default(),
            || Ok(())
        )
        .is_err());
        assert_eq!(fs_err::read(&output).unwrap(), b"original");
        assert!(write_output(
            &output,
            &dom(true),
            OutputKind::Rbxl,
            rbx_binary::CompressionType::default(),
            || anyhow::bail!("input changed")
        )
        .is_err());
        assert_eq!(fs_err::read(&output).unwrap(), b"original");
        assert_eq!(
            fs_err::read(&unrelated).unwrap(),
            b"preserve other invocation"
        );
        fs_err::remove_file(unrelated).unwrap();
        no_temps(dir.path());
    }

    #[test]
    fn failed_replace_preserves_existing_directory_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output.rbxl");
        fs_err::create_dir(&output).unwrap();
        fs_err::write(output.join("sentinel"), b"keep").unwrap();
        assert!(write_output(
            &output,
            &dom(true),
            OutputKind::Rbxl,
            rbx_binary::CompressionType::default(),
            || Ok(())
        )
        .is_err());
        assert_eq!(fs_err::read(output.join("sentinel")).unwrap(), b"keep");
        no_temps(dir.path());
    }
}
