use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context as _};
use indexmap::IndexMap;
use memofs::{IoResultExt as _, Vfs};
use rbx_dom_weak::{
    types::{Attributes, Tags, Variant},
    Ustr,
};
use serde::{Deserialize, Serialize};

use super::Middleware;
use crate::{
    project::RuleDefaultMask,
    resolution::UnresolvedValue,
    snapshot::{InstanceContext, InstanceSnapshot},
    syncback::SyncbackSnapshot,
    variant_eq::variant_eq,
};

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct RuleOwner {
    pub(crate) path: PathBuf,
    pub(crate) class_name: Option<Ustr>,
    pub(crate) mask: RuleDefaultMask,
}

impl RuleOwner {
    fn matches(&self, path: &Path) -> bool {
        path == self.path
            || (path.parent() == Some(self.path.as_path())
                && path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(super::is_init_file_name))
    }
}

/// Shared fallback values for instances owned by matching physical paths.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleDefaults {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<Ustr, UnresolvedValue>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, UnresolvedValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<Ustr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_unknown_instances: Option<bool>,
}

impl RuleDefaults {
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
            && self.attributes.is_empty()
            && self.tags.is_none()
            && self.class_name.is_none()
            && self.ignore_unknown_instances.is_none()
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        for key in self.properties.keys() {
            ensure!(
                !matches!(
                    key.as_str(),
                    "Name" | "Parent" | "Source" | "Value" | "Contents"
                ),
                "syncRules defaults cannot replace structural or middleware-owned property {key}"
            );
        }
        ensure!(
            self.tags.is_none() || !self.properties.contains_key(&"Tags".into()),
            "syncRules defaults cannot specify both tags and properties.Tags"
        );
        ensure!(
            !self.properties.contains_key(&"Attributes".into()),
            "syncRules defaults must specify attributes per key, not properties.Attributes"
        );
        Ok(())
    }
}

/// Snapshot-local authority, not a cache of source contents or a wire extension.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct RuleDefaultState {
    pub(crate) path: PathBuf,
    pub(crate) mask: RuleDefaultMask,
    pub(crate) properties: BTreeMap<Ustr, Variant>,
    pub(crate) attributes: BTreeMap<String, Variant>,
    pub(crate) class_name: Option<Ustr>,
    pub(crate) ignore: Option<bool>,
    pub(crate) explicit_properties: BTreeSet<Ustr>,
    pub(crate) explicit_attributes: BTreeSet<String>,
    pub(crate) explicit_class: bool,
    pub(crate) explicit_ignore: Option<bool>,
    // Only fallback keys are retained. Never duplicate Source or whole subtrees.
    originals: BTreeMap<Ustr, Option<Variant>>,
    added_attributes: BTreeSet<String>,
    original_class: Option<Ustr>,
    original_ignore: Option<bool>,
}

pub(super) fn apply(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
    middleware: Middleware,
    snapshot: &mut InstanceSnapshot,
) -> anyhow::Result<()> {
    if !context
        .sync_rules
        .iter()
        .any(|rule| rule.defaults.is_some() && rule.matches(path))
    {
        return Ok(());
    }
    let prepared = snapshot.metadata.rule_defaults.take().unwrap_or_default();
    let mut state = RuleDefaultState {
        path: path.to_owned(),
        original_class: prepared.original_class,
        originals: prepared.originals,
        ..Default::default()
    };
    state
        .explicit_properties
        .extend(snapshot.properties.keys().copied());
    // Script RunContext is emitted by routing unless metadata explicitly sets it.
    if matches!(snapshot.class_name.as_str(), "Script" | "LocalScript")
        && !middleware.handles_own_properties()
    {
        state.explicit_properties.remove(&"RunContext".into());
    }
    if let Some(Variant::Attributes(attrs)) = snapshot.properties.get(&"Attributes".into()) {
        state
            .explicit_attributes
            .extend(attrs.iter().map(|(key, _)| key.clone()));
    }
    state.explicit_class = middleware.handles_own_properties();
    if let Some(owner) = context
        .rule_owner
        .as_ref()
        .filter(|owner| owner.matches(path))
    {
        state.explicit_class |= owner.class_name.is_some();
        merge_mask(&mut state.mask, &owner.mask);
    }
    let mut meta_paths: Vec<_> = snapshot
        .metadata
        .relevant_paths
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".meta.json") || s.ends_with(".meta.jsonc"))
        })
        .cloned()
        .collect();
    // Init middleware transfers directory metadata; include the adjacent choices
    // already consumed by its source reader even when that transfer dropped them.
    for suffix in ["json", "jsonc"] {
        if middleware == Middleware::Dir {
            break;
        }
        let candidate = path.with_file_name(format!("{name}.meta.{suffix}"));
        if (middleware.handles_own_properties() || middleware.is_dir())
            && !snapshot.metadata.relevant_paths.contains(&candidate)
        {
            snapshot.metadata.relevant_paths.push(candidate.clone());
        }
        if !meta_paths.contains(&candidate) {
            meta_paths.push(candidate);
        }
    }
    for meta_path in meta_paths {
        if let Some(bytes) = vfs.read(&meta_path).with_not_found()? {
            let value: serde_json::Value = crate::json::from_slice(&bytes)?;
            if let Some(mask) = value.get("ruleDefaults") {
                let mask: RuleDefaultMask = serde_json::from_value(mask.clone())
                    .with_context(|| format!("Invalid ruleDefaults in {}", meta_path.display()))?;
                merge_mask(&mut state.mask, &mask);
            }
            if !middleware.handles_own_properties() {
                state.explicit_class |= value.get("className").is_some_and(|v| !v.is_null());
                if let Some(ignore) = value
                    .get("ignoreUnknownInstances")
                    .and_then(|v| v.as_bool())
                {
                    state.explicit_ignore = Some(ignore);
                }
                if value
                    .get("properties")
                    .and_then(|p| p.get("RunContext"))
                    .is_some()
                {
                    state.explicit_properties.insert("RunContext".into());
                }
            }
        }
    }
    let mut unresolved = BTreeMap::new();
    let mut attributes = BTreeMap::new();
    let mut tags = None;
    for (index, rule) in context
        .sync_rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.matches(path))
    {
        let Some(defaults) = &rule.defaults else {
            continue;
        };
        defaults.validate().with_context(|| {
            format!(
                "syncRules[{index}] in {} for {}",
                rule.base_path.display(),
                path.display()
            )
        })?;
        for (key, value) in &defaults.properties {
            if key.as_str() == "Tags" {
                if tags.is_none() {
                    tags = Some(value.clone());
                }
            } else {
                unresolved.entry(*key).or_insert_with(|| value.clone());
            }
        }
        if tags.is_none() {
            if let Some(values) = &defaults.tags {
                let mut value = Tags::new();
                for tag in values {
                    value.push(tag);
                }
                tags = Some(UnresolvedValue::from_variant_unambiguous(value.into()));
            }
        }
        for (key, value) in &defaults.attributes {
            attributes
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
        state.class_name = state.class_name.or(defaults.class_name);
        state.ignore = state.ignore.or(defaults.ignore_unknown_instances);
    }
    if let Some(tags) = tags {
        unresolved.insert("Tags".into(), tags);
    }
    let target_class = if !state.explicit_class && !state.mask.disable_class_name {
        state.class_name.unwrap_or(snapshot.class_name)
    } else {
        snapshot.class_name
    };
    if target_class != snapshot.class_name {
        ensure!(
            snapshot.class_name == "Folder"
                || (matches!(snapshot.class_name.as_str(), "Script" | "LocalScript")
                    && matches!(target_class.as_str(), "Script" | "LocalScript")),
            "Invalid syncRules className at {}: {} -> {}",
            path.display(),
            snapshot.class_name,
            target_class
        );
    }
    for (key, value) in unresolved {
        if state.mask.disabled_properties.contains(&key) {
            continue;
        }
        ensure!(
            target_class != "LocalScript"
                || key.as_str() != "RunContext"
                || state.mask.disabled_properties.contains(&key),
            "LocalScript rule defaults cannot specify RunContext at {}",
            path.display()
        );
        let value = value
            .resolve(&target_class, &key)
            .with_context(|| format!("Invalid syncRules default {key} at {}", path.display()))?;
        state.properties.insert(key, value);
    }
    for (key, value) in attributes {
        state.attributes.insert(
            key.clone(),
            value.resolve_unambiguous().with_context(|| {
                format!("Invalid syncRules attribute {key} at {}", path.display())
            })?,
        );
    }
    snapshot.metadata.context = context.clone();
    install(snapshot, &mut state)?;
    snapshot.metadata.rule_defaults = Some(Box::new(state));
    Ok(())
}

fn merge_mask(target: &mut RuleDefaultMask, mask: &RuleDefaultMask) {
    for key in &mask.disabled_properties {
        if !target.disabled_properties.contains(key) {
            target.disabled_properties.push(*key);
        }
    }
    for key in &mask.disabled_attributes {
        if !target.disabled_attributes.contains(key) {
            target.disabled_attributes.push(key.clone());
        }
    }
    target.disable_class_name |= mask.disable_class_name;
    target.disable_ignore_unknown_instances |= mask.disable_ignore_unknown_instances;
}

fn install(snapshot: &mut InstanceSnapshot, state: &mut RuleDefaultState) -> anyhow::Result<()> {
    if let Some(class) = state
        .class_name
        .filter(|_| !state.explicit_class && !state.mask.disable_class_name)
    {
        state.original_class.get_or_insert(snapshot.class_name);
        snapshot.class_name = class;
        if class == "LocalScript" {
            ensure!(
                !state.explicit_properties.contains(&"RunContext".into()),
                "LocalScript cannot specify RunContext at {}",
                state.path.display()
            );
            let previous = snapshot.properties.remove(&"RunContext".into());
            state
                .originals
                .entry("RunContext".into())
                .or_insert(previous);
        }
    }
    if let Some(ignore) = state
        .ignore
        .filter(|_| state.explicit_ignore.is_none() && !state.mask.disable_ignore_unknown_instances)
    {
        state.original_ignore = Some(snapshot.metadata.ignore_unknown_instances);
        snapshot.metadata.ignore_unknown_instances = ignore;
    }
    for (key, value) in &state.properties {
        if !state.explicit_properties.contains(key) && !state.mask.disabled_properties.contains(key)
        {
            state
                .originals
                .insert(*key, snapshot.properties.insert(*key, value.clone()));
        }
    }
    let mut attrs = match snapshot.properties.get(&"Attributes".into()) {
        Some(Variant::Attributes(attrs)) => attrs.clone(),
        _ => Attributes::new(),
    };
    for (key, value) in &state.attributes {
        if !state.explicit_attributes.contains(key) && !state.mask.disabled_attributes.contains(key)
        {
            attrs.insert(key.clone(), value.clone());
            state.added_attributes.insert(key.clone());
        }
    }
    if !state.added_attributes.is_empty() {
        snapshot
            .properties
            .insert("Attributes".into(), attrs.into());
    }
    Ok(())
}

pub(super) fn project_mask(
    snapshot: &mut InstanceSnapshot,
    mask: &RuleDefaultMask,
) -> anyhow::Result<()> {
    let Some(mut state) = snapshot.metadata.rule_defaults.take() else {
        return Ok(());
    };
    for (key, value) in std::mem::take(&mut state.originals) {
        match value {
            Some(value) => {
                snapshot.properties.insert(key, value);
            }
            None => {
                snapshot.properties.remove(&key);
            }
        }
    }
    if let Some(class) = state.original_class.take() {
        snapshot.class_name = class;
    }
    if let Some(ignore) = state.original_ignore.take() {
        snapshot.metadata.ignore_unknown_instances = ignore;
    }
    if let Some(Variant::Attributes(attrs)) = snapshot.properties.get_mut(&"Attributes".into()) {
        for key in std::mem::take(&mut state.added_attributes) {
            attrs.remove(key);
        }
    }
    merge_mask(&mut state.mask, mask);
    install(snapshot, &mut state)?;
    snapshot.metadata.rule_defaults = Some(state);
    Ok(())
}

pub(super) fn syncback_state(
    snapshot: &SyncbackSnapshot,
) -> anyhow::Result<Option<Box<RuleDefaultState>>> {
    if let Some(state) = snapshot
        .old_inst()
        .and_then(|old| old.metadata().rule_defaults.clone())
    {
        return Ok(Some(state));
    }
    let context = super::lua::destination_context(snapshot)?;
    if !context
        .sync_rules
        .iter()
        .any(|rule| rule.defaults.is_some())
    {
        return Ok(None);
    }
    let middleware = snapshot
        .middleware
        .unwrap_or(crate::syncback::get_best_middleware(snapshot));
    let path = if middleware.is_dir() && middleware != Middleware::Dir {
        if snapshot.vfs().exists(&snapshot.path)? {
            super::get_dir_middleware(snapshot.vfs(), &snapshot.path)?.2
        } else {
            snapshot.path.join(match middleware {
                Middleware::ServerScriptDir => "init.server.luau",
                Middleware::ClientScriptDir => "init.client.luau",
                Middleware::PluginScriptDir => "init.plugin.luau",
                Middleware::CsvDir => "init.csv",
                _ => "init.luau",
            })
        }
    } else {
        snapshot.path.clone()
    };
    // New instances have no persisted explicit authority. Evaluate the same
    // physical fallback without treating the imported DOM's values as file data.
    let mut baseline = InstanceSnapshot::new()
        .name(&snapshot.new_inst().name)
        .class_name(snapshot.new_inst().class);
    apply(
        &context,
        snapshot.vfs(),
        &path,
        &snapshot.new_inst().name,
        middleware,
        &mut baseline,
    )?;
    Ok(baseline.metadata.rule_defaults)
}

/// Remove only values known to remain supplied by rules, and record individual
/// removals. Equality alone never erases an explicit source override.
pub(super) fn rewrite_metadata(
    snapshot: &SyncbackSnapshot,
    properties: &mut IndexMap<Ustr, UnresolvedValue>,
    attributes: &mut IndexMap<String, UnresolvedValue>,
    mask: &mut RuleDefaultMask,
    ignore: &mut Option<bool>,
) -> anyhow::Result<Option<Box<RuleDefaultState>>> {
    if let Some(old) = snapshot.old_inst() {
        for path in &old.metadata().relevant_paths {
            if !path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".meta.json") || s.ends_with(".meta.jsonc"))
            {
                continue;
            }
            if let Some(bytes) = snapshot.vfs().read(path).with_not_found()? {
                let value: serde_json::Value = crate::json::from_slice(&bytes)?;
                if let Some(value) = value.get("ruleDefaults") {
                    merge_mask(mask, &serde_json::from_value(value.clone())?);
                }
            }
        }
    }
    let Some(state) = syncback_state(snapshot)? else {
        return Ok(None);
    };
    merge_mask(mask, &state.mask);
    let instance = snapshot.new_inst();
    let ignored =
        crate::syncback::get_property_filter(snapshot.project(), instance).unwrap_or_default();
    for (key, fallback) in &state.properties {
        if state.mask.disabled_properties.contains(key) || ignored.contains(key) {
            continue;
        }
        if let Some(value) = instance.properties.get(key) {
            if !state.explicit_properties.contains(key) && variant_eq(value, fallback) {
                properties.shift_remove(key);
            } else {
                properties.insert(
                    *key,
                    UnresolvedValue::from_variant(value.clone(), &instance.class, key),
                );
            }
        } else {
            properties.shift_remove(key);
            if !mask.disabled_properties.contains(key) {
                mask.disabled_properties.push(*key);
            }
        }
    }
    let actual = match instance.properties.get(&"Attributes".into()) {
        Some(Variant::Attributes(attrs)) => Some(attrs),
        _ => None,
    };
    for (key, fallback) in &state.attributes {
        if state.mask.disabled_attributes.contains(key) || ignored.contains(&"Attributes".into()) {
            continue;
        }
        if let Some(value) = actual.and_then(|attrs| attrs.get(key.as_str())) {
            if !state.explicit_attributes.contains(key) && variant_eq(value, fallback) {
                attributes.shift_remove(key);
            } else {
                attributes.insert(
                    key.clone(),
                    UnresolvedValue::from_variant_unambiguous(value.clone()),
                );
            }
        } else {
            attributes.shift_remove(key);
            if !mask.disabled_attributes.contains(key) {
                mask.disabled_attributes.push(key.clone());
            }
        }
    }
    if state.ignore.is_some() && !state.mask.disable_ignore_unknown_instances {
        *ignore = state.explicit_ignore;
    }
    properties.sort_by(|a, _, b, _| a.as_str().cmp(b.as_str()));
    attributes.sort_keys();
    mask.disabled_properties.sort();
    mask.disabled_properties.dedup();
    mask.disabled_attributes.sort();
    mask.disabled_attributes.dedup();
    Ok(Some(state))
}

pub(super) fn preserve_metadata_paths(
    snapshot: &SyncbackSnapshot,
    fs: &mut crate::syncback::FsSnapshot,
) -> anyhow::Result<()> {
    if syncback_state(snapshot)?.is_none() {
        return Ok(());
    }
    let paths: Vec<_> = fs
        .added_files()
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".meta.json"))
        })
        .map(Path::to_owned)
        .collect();
    for path in paths {
        let destination = super::lua::metadata_destination(snapshot.vfs(), &path)?;
        if destination != path {
            fs.move_added_file(&path, destination);
        }
    }
    Ok(())
}

pub(super) fn model_mask(
    snapshot: &SyncbackSnapshot,
    mask: &RuleDefaultMask,
    fs: &mut crate::syncback::FsSnapshot,
) -> anyhow::Result<()> {
    if syncback_state(snapshot)?.is_none() {
        return Ok(());
    }
    let context = super::lua::destination_context(snapshot)?;
    let rule = context.get_user_sync_rule(&snapshot.path).or_else(|| {
        super::default_sync_rules()
            .iter()
            .find(|r| r.matches(&snapshot.path))
    });
    let name = rule
        .map(|r| r.file_name_for_path(&snapshot.path))
        .transpose()?
        .unwrap_or(&snapshot.new_inst().name);
    let path = snapshot.path.with_file_name(format!("{name}.meta.json"));
    let path = super::lua::metadata_destination(snapshot.vfs(), &path)?;
    let mut value: serde_json::Value = match snapshot.vfs().read(&path).with_not_found()? {
        Some(bytes) => crate::json::from_slice(&bytes)?,
        None if mask.is_empty() => return Ok(()),
        None => serde_json::json!({}),
    };
    let object = value
        .as_object_mut()
        .context("Model ruleDefaults sidecar must be an object")?;
    let previous = object.get("ruleDefaults").cloned();
    if mask.is_empty() {
        object.remove("ruleDefaults");
    } else {
        object.insert("ruleDefaults".into(), serde_json::to_value(mask)?);
    }
    if object.get("ruleDefaults") != previous.as_ref() {
        fs.add_file(&path, serde_json::to_vec_pretty(&value)?);
    }
    Ok(())
}

/// Clone just this model subtree, retaining referents and reference markers.
/// Descendants have no independent physical binding and keep all their data.
pub(super) fn model_projection(
    snapshot: &SyncbackSnapshot,
) -> anyhow::Result<Option<rbx_dom_weak::WeakDom>> {
    let Some(state) = syncback_state(snapshot)? else {
        return Ok(None);
    };
    use rbx_dom_weak::{InstanceBuilder, WeakDom};
    let mut tree = WeakDom::new(InstanceBuilder::new("DataModel"));
    let mut queue = std::collections::VecDeque::from([(snapshot.new, tree.root_ref())]);
    while let Some((id, parent)) = queue.pop_front() {
        let instance = snapshot.new_tree().get_by_ref(id).unwrap();
        tree.insert(
            parent,
            InstanceBuilder::new(instance.class)
                .with_name(&instance.name)
                .with_referent(id)
                .with_properties(instance.properties.clone()),
        );
        queue.extend(instance.children().iter().map(|child| (*child, id)));
    }
    let root = tree.get_by_ref_mut(snapshot.new).unwrap();
    for (key, fallback) in &state.properties {
        if !state.explicit_properties.contains(key)
            && !state.mask.disabled_properties.contains(key)
            && root
                .properties
                .get(key)
                .is_some_and(|value| variant_eq(value, fallback))
        {
            root.properties.remove(key);
        }
    }
    if let Some(Variant::Attributes(attrs)) = root.properties.get_mut(&"Attributes".into()) {
        for (key, fallback) in &state.attributes {
            if !state.explicit_attributes.contains(key)
                && !state.mask.disabled_attributes.contains(key)
                && attrs
                    .get(key.as_str())
                    .is_some_and(|value| variant_eq(value, fallback))
            {
                attrs.remove(key.as_str());
            }
        }
        if attrs.is_empty() {
            root.properties.remove(&"Attributes".into());
        }
    }
    Ok(Some(tree))
}

pub(super) fn write_model_mask(
    snapshot: &SyncbackSnapshot,
    fs: &mut crate::syncback::FsSnapshot,
) -> anyhow::Result<()> {
    let mut mask = RuleDefaultMask::default();
    rewrite_metadata(
        snapshot,
        &mut IndexMap::new(),
        &mut IndexMap::new(),
        &mut mask,
        &mut None,
    )?;
    model_mask(snapshot, &mask, fs)
}

pub(super) fn validate_binary_projection(
    snapshot: &SyncbackSnapshot,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let Some(state) = syncback_state(snapshot)? else {
        return Ok(());
    };
    let decoded = rbx_binary::from_reader(bytes)?;
    let root = decoded
        .root()
        .children()
        .first()
        .and_then(|id| decoded.get_by_ref(*id))
        .context("Encoded model has no root")?;
    let actual = snapshot.new_inst();
    for (key, fallback) in &state.properties {
        if !state.explicit_properties.contains(key)
            && !state.mask.disabled_properties.contains(key)
            && actual
                .properties
                .get(key)
                .is_some_and(|value| variant_eq(value, fallback))
        {
            ensure!(!root.properties.contains_key(key),
                "Cannot syncback {}: binary property columns would materialize inherited rule default {key}; use an XML/JSON source or an explicit model override", snapshot.path.display());
        }
    }
    let actual_attrs = match actual.properties.get(&"Attributes".into()) {
        Some(Variant::Attributes(attrs)) => Some(attrs),
        _ => None,
    };
    let encoded_attrs = match root.properties.get(&"Attributes".into()) {
        Some(Variant::Attributes(attrs)) => Some(attrs),
        _ => None,
    };
    for (key, fallback) in &state.attributes {
        if !state.explicit_attributes.contains(key)
            && !state.mask.disabled_attributes.contains(key)
            && actual_attrs
                .and_then(|attrs| attrs.get(key.as_str()))
                .is_some_and(|value| variant_eq(value, fallback))
        {
            ensure!(
                encoded_attrs
                    .and_then(|attrs| attrs.get(key.as_str()))
                    .is_none(),
                "Cannot syncback {}: binary encoding would materialize inherited attribute {key}",
                snapshot.path.display()
            );
        }
    }
    Ok(())
}

/// Class-aware metadata must resolve against a rule-selected native/directory
/// class. Inspect explicit class/mask layers before changing the intrinsic class.
pub(super) fn prepare_class(
    snapshot: &mut InstanceSnapshot,
    vfs: &Vfs,
    path: &Path,
    meta_paths: &[PathBuf],
) -> anyhow::Result<()> {
    let context = &snapshot.metadata.context;
    if !context
        .sync_rules
        .iter()
        .any(|r| r.matches(path) && r.defaults.is_some())
    {
        return Ok(());
    }
    let owner = context
        .rule_owner
        .as_ref()
        .filter(|owner| owner.matches(path));
    if let Some(class) = owner.and_then(|owner| owner.class_name) {
        ensure!(
            snapshot.class_name == "Folder" || snapshot.class_name == class,
            "Project className conflicts with native source at {}: {} -> {}",
            path.display(),
            snapshot.class_name,
            class
        );
    }
    let class = context
        .sync_rules
        .iter()
        .filter(|r| r.matches(path))
        .filter_map(|r| r.defaults.as_ref()?.class_name)
        .next();
    let class = owner.and_then(|owner| owner.class_name).or_else(|| {
        if owner.is_some_and(|owner| owner.mask.disable_class_name) {
            None
        } else {
            class
        }
    });
    let Some(class) = class else { return Ok(()) };
    for meta_path in meta_paths {
        if let Some(bytes) = vfs.read(meta_path).with_not_found()? {
            let value: serde_json::Value = crate::json::from_slice(&bytes)?;
            if value.get("className").is_some_and(|v| !v.is_null())
                || value
                    .get("ruleDefaults")
                    .and_then(|v| v.get("disableClassName"))
                    .and_then(|v| v.as_bool())
                    == Some(true)
                    && owner.and_then(|owner| owner.class_name).is_none()
            {
                return Ok(());
            }
        }
    }
    ensure!(
        snapshot.class_name == class
            || snapshot.class_name == "Folder"
            || (matches!(snapshot.class_name.as_str(), "Script" | "LocalScript")
                && matches!(class.as_str(), "Script" | "LocalScript")),
        "Invalid syncRules className at {}: {} -> {}",
        path.display(),
        snapshot.class_name,
        class
    );
    let state = snapshot
        .metadata
        .rule_defaults
        .get_or_insert_with(Default::default);
    state.original_class.get_or_insert(snapshot.class_name);
    if class == "LocalScript" {
        state
            .originals
            .entry("RunContext".into())
            .or_insert_with(|| snapshot.properties.get(&"RunContext".into()).cloned());
    }
    snapshot.class_name = class;
    Ok(())
}
