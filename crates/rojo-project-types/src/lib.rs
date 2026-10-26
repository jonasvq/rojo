//! Reusable Rojo project tree types without filesystem, CLI or transport runtime.

use rbx_dom_weak::Ustr;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod authority;
#[doc(hidden)]
pub mod path_serializer;
mod resolution;
pub use authority::{ClassAuthorityPolicy, PropertyAuthority, StructureAuthority, SyncAuthority};
pub use resolution::{AmbiguousValue, UnresolvedValue};

pub const REF_POINTER_ATTRIBUTE_PREFIX: &str = "Rojo_Target_";

fn optional_mask_is_empty(mask: &Option<Box<RuleDefaultMask>>) -> bool {
    mask.as_deref().is_none_or(RuleDefaultMask::is_empty)
}

impl ProjectNode {
    pub fn rule_default_mask(&self) -> &RuleDefaultMask {
        self.rule_defaults
            .as_deref()
            .unwrap_or(RuleDefaultMask::EMPTY)
    }
}

/// Per-key opt-outs from matching filesystem rule defaults.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleDefaultMask {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_properties: Vec<Ustr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disable_class_name: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disable_ignore_unknown_instances: bool,
}

impl RuleDefaultMask {
    pub const EMPTY: &'static Self = &Self {
        disabled_properties: Vec::new(),
        disabled_attributes: Vec::new(),
        disable_class_name: false,
        disable_ignore_unknown_instances: false,
    };
    pub fn is_empty(&self) -> bool {
        self.disabled_properties.is_empty()
            && self.disabled_attributes.is_empty()
            && !self.disable_class_name
            && !self.disable_ignore_unknown_instances
    }
}

/// Explicit project-format semantics. Omitted versions retain V1 behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum ProjectFormatVersion {
    V1,
    V2,
}

impl TryFrom<u32> for ProjectFormatVersion {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            _ => Err(format!(
                "Unsupported project $version {value}; expected 1 or 2"
            )),
        }
    }
}

impl From<ProjectFormatVersion> for u32 {
    fn from(value: ProjectFormatVersion) -> Self {
        match value {
            ProjectFormatVersion::V1 => 1,
            ProjectFormatVersion::V2 => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OptionalPathNode {
    #[serde(serialize_with = "crate::path_serializer::serialize_absolute")]
    pub optional: PathBuf,
}

impl OptionalPathNode {
    pub fn new(optional: PathBuf) -> Self {
        OptionalPathNode { optional }
    }
}

/// Describes a path that is either optional or required
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathNode {
    Required(#[serde(serialize_with = "crate::path_serializer::serialize_absolute")] PathBuf),
    Multiple(MultiplePaths),
    Optional(OptionalPathNode),
}

/// A nonempty list of directory sources for read-only sourcemap aggregation.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "Vec<PathBuf>")]
pub struct MultiplePaths(Vec<PathBuf>);

impl Serialize for MultiplePaths {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        crate::path_serializer::serialize_vec_absolute(&self.0, serializer)
    }
}

impl MultiplePaths {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<Self> {
        anyhow::ensure!(!paths.is_empty(), "$path arrays must not be empty");
        Ok(Self(paths))
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }
}

impl TryFrom<Vec<PathBuf>> for MultiplePaths {
    type Error = anyhow::Error;

    fn try_from(paths: Vec<PathBuf>) -> Result<Self, Self::Error> {
        Self::new(paths)
    }
}

impl From<MultiplePaths> for Vec<PathBuf> {
    fn from(paths: MultiplePaths) -> Self {
        paths.0
    }
}

impl PathNode {
    /// Returns the primary first source path, whether required or optional.
    /// Use [`Self::paths`] when aggregating multiple sources.
    #[inline]
    pub fn path(&self) -> &Path {
        match self {
            PathNode::Required(pathbuf) => pathbuf,
            PathNode::Optional(OptionalPathNode { optional }) => optional,
            PathNode::Multiple(paths) => &paths.paths()[0],
        }
    }

    /// All source paths, in input order. `path()` returns the primary first path.
    pub fn paths(&self) -> &[PathBuf] {
        match self {
            Self::Required(path) => std::slice::from_ref(path),
            Self::Optional(path) => std::slice::from_ref(&path.optional),
            Self::Multiple(paths) => paths.paths(),
        }
    }

    /// Returns whether this `PathNode` is optional or not.
    #[inline]
    pub fn is_optional(&self) -> bool {
        matches!(self, PathNode::Optional(_))
    }
}

/// Describes an instance and its descendants in a project.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProjectNode {
    #[serde(
        rename = "$ruleDefaults",
        default,
        skip_serializing_if = "optional_mask_is_empty"
    )]
    pub rule_defaults: Option<Box<RuleDefaultMask>>,
    /// If set, defines the ClassName of the described instance.
    ///
    /// `$className` MUST be set if `$path` is not set.
    ///
    /// With `$path`, the source must be a Folder or have the same ClassName.
    #[serde(rename = "$className", skip_serializing_if = "Option::is_none")]
    pub class_name: Option<Ustr>,

    /// If set, defines an ID for the described Instance that can be used
    /// to refer to it for the purpose of referent properties.
    #[serde(rename = "$id", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Contains all of the children of the described instance.
    #[serde(flatten)]
    pub children: BTreeMap<String, ProjectNode>,

    /// The properties that will be assigned to the resulting instance.
    ///
    // TODO: Is this legal to set if $path is set?
    #[serde(
        rename = "$properties",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub properties: BTreeMap<Ustr, UnresolvedValue>,

    #[serde(
        rename = "$attributes",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub attributes: BTreeMap<String, UnresolvedValue>,

    /// Defines the behavior when Rojo encounters unknown instances in Roblox
    /// Studio during live sync. `$ignoreUnknownInstances` should be considered
    /// a large hammer and used with care.
    ///
    /// If set to `true`, those instances will be left alone. This may cause
    /// issues when files that turn into instances are removed while Rojo is not
    /// running.
    ///
    /// If set to `false`, Rojo will destroy any instances it does not
    /// recognize.
    ///
    /// If unset, its default value depends on other settings:
    /// - If `$path` is not set, defaults to `true`
    /// - If `$path` is set, defaults to `false`
    #[serde(
        rename = "$ignoreUnknownInstances",
        skip_serializing_if = "Option::is_none"
    )]
    pub ignore_unknown_instances: Option<bool>,

    /// Inherited default for this node and its descendants. `false` ends
    /// inherited preservation; explicit per-instance metadata still wins.
    #[serde(
        rename = "$ignoreUnknownDescendants",
        skip_serializing_if = "Option::is_none"
    )]
    pub ignore_unknown_descendants: Option<bool>,

    /// Defines that this instance should come from the given file path. This
    /// path can point to any file type supported by Rojo, including Lua files
    /// (`.lua`), Roblox models (`.rbxm`, `.rbxmx`), and localization table
    /// spreadsheets (`.csv`).
    #[serde(rename = "$path", skip_serializing_if = "Option::is_none")]
    pub path: Option<PathNode>,
}
