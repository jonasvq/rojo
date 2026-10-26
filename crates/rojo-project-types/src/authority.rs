use std::collections::HashSet;

use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};

/// Authority for managed property edits, including Script.Source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyAuthority {
    Filesystem,
    Studio,
    TwoWay,
}

/// Structural changes require a separate opt-in from property edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureAuthority {
    #[default]
    Filesystem,
    TwoWay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassAuthorityPolicy {
    pub class_name: String,
    pub properties: PropertyAuthority,
    #[serde(default)]
    pub structure: StructureAuthority,
}

/// Validated class policies. A missing match retains legacy behavior; it does
/// not grant Studio property or structure permissions.
///
/// These reusable types do not themselves enable live writes. Connection consent,
/// ownership, supported serialization and an exact fresh base are also required.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(
    try_from = "Vec<ClassAuthorityPolicy>",
    into = "Vec<ClassAuthorityPolicy>"
)]
pub struct SyncAuthority(Vec<ClassAuthorityPolicy>);

impl SyncAuthority {
    pub fn new(policies: Vec<ClassAuthorityPolicy>) -> anyhow::Result<Self> {
        let database = rbx_reflection_database::get()?;
        let mut seen = HashSet::new();
        for policy in &policies {
            ensure!(
                database.classes.contains_key(policy.class_name.as_str()),
                "Unknown syncAuthority class {:?}",
                policy.class_name
            );
            ensure!(
                seen.insert(&policy.class_name),
                "Duplicate syncAuthority class {:?}",
                policy.class_name
            );
        }
        Ok(Self(policies))
    }

    pub fn policies(&self) -> &[ClassAuthorityPolicy] {
        &self.0
    }

    /// Select the exact class or nearest ancestor, regardless of list order.
    /// Unknown instance classes are errors rather than implicit permissions.
    pub fn resolve(&self, class_name: &str) -> anyhow::Result<Option<&ClassAuthorityPolicy>> {
        let database = rbx_reflection_database::get()?;
        let mut class = database
            .classes
            .get(class_name)
            .with_context(|| format!("Unknown managed instance class {class_name:?}"))?;
        loop {
            if let Some(policy) = self.0.iter().find(|policy| policy.class_name == class.name) {
                return Ok(Some(policy));
            }
            let Some(parent) = &class.superclass else {
                return Ok(None);
            };
            class = database
                .classes
                .get(parent)
                .context("Reflection database contains an unknown superclass")?;
        }
    }
}

impl TryFrom<Vec<ClassAuthorityPolicy>> for SyncAuthority {
    type Error = anyhow::Error;

    fn try_from(policies: Vec<ClassAuthorityPolicy>) -> Result<Self, Self::Error> {
        Self::new(policies)
    }
}

impl From<SyncAuthority> for Vec<ClassAuthorityPolicy> {
    fn from(authority: SyncAuthority) -> Self {
        authority.0
    }
}
