use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    glob::{Glob, IgnorableGlob},
    path_serializer,
    project::ProjectNode,
    snapshot_middleware::{emit_legacy_scripts_default, Middleware},
    RojoRef,
};

/// Rojo-specific metadata that can be associated with an instance or a snapshot
/// of an instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceMetadata {
    /// Effective authority, distinct from the incoming context used when a
    /// project is reloaded. Private protocol state; never grants client rights.
    #[doc(hidden)]
    #[serde(skip)]
    pub effective_sync_authority: Option<Arc<crate::SyncAuthority>>,
    /// Rule authority retained for reversible syncback; never sent on the wire.
    #[doc(hidden)]
    #[serde(skip)]
    pub rule_defaults: Option<Box<crate::snapshot_middleware::RuleDefaultState>>,
    /// Whether instances not present in the source should be ignored when
    /// live-syncing. This is useful when there are instances that Rojo does not
    /// manage.
    pub ignore_unknown_instances: bool,

    /// If a change occurs to this instance, the instigating source is what
    /// should be run through the snapshot functions to regenerate it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instigating_source: Option<InstigatingSource>,

    /// The paths that, when changed, could cause the function that generated
    /// this snapshot to generate a different snapshot. Paths should be included
    /// even if they don't exist, since the presence of a file can change the
    /// outcome of a snapshot function.
    ///
    /// For example, a file named foo.lua might have these relevant paths:
    /// - foo.lua
    /// - foo.meta.json (even if this file doesn't exist!)
    ///
    /// A directory named bar/ might have these:
    /// - bar/
    /// - bar/init.meta.json
    /// - bar/init.lua
    /// - bar/init.server.lua
    /// - bar/init.client.lua
    /// - bar/default.project.json
    ///
    /// This path is used to make sure that file changes update all instances
    /// that may need updates.
    // TODO: Change this to be a SmallVec for performance in common cases?
    #[serde(serialize_with = "path_serializer::serialize_vec_absolute")]
    pub relevant_paths: Vec<PathBuf>,

    /// Contains information about this instance that should persist between
    /// snapshot invocations and is generally inherited.
    ///
    /// If an instance has a piece of context attached to it, then the next time
    /// that instance's instigating source is snapshotted directly, the same
    /// context will be passed into it.
    pub context: InstanceContext,

    /// Indicates the ID used for Ref properties pointing to this Instance.
    pub specified_id: Option<RojoRef>,

    /// The Middleware that was used to create this Instance. Should generally
    /// not be `None` except if the snapshotting process is not completed.
    pub middleware: Option<Middleware>,

    /// A schema provided via a JSON file, if one exists. Will be `None` for
    /// all non-JSON middleware.
    pub schema: Option<String>,
}

impl InstanceMetadata {
    pub fn new() -> Self {
        Self {
            effective_sync_authority: None,
            rule_defaults: None,
            ignore_unknown_instances: false,
            instigating_source: None,
            relevant_paths: Vec::new(),
            context: InstanceContext::default(),
            specified_id: None,
            middleware: None,
            schema: None,
        }
    }

    pub fn ignore_unknown_instances(self, ignore_unknown_instances: bool) -> Self {
        Self {
            ignore_unknown_instances,
            ..self
        }
    }

    pub fn instigating_source(self, instigating_source: impl Into<InstigatingSource>) -> Self {
        Self {
            instigating_source: Some(instigating_source.into()),
            ..self
        }
    }

    pub fn relevant_paths(self, relevant_paths: Vec<PathBuf>) -> Self {
        Self {
            relevant_paths,
            ..self
        }
    }

    pub fn context(self, context: &InstanceContext) -> Self {
        Self {
            effective_sync_authority: context.sync_authority.clone(),
            ignore_unknown_instances: self.ignore_unknown_instances
                || context.ignore_unknown_descendants == Some(true),
            context: context.clone(),
            ..self
        }
    }

    pub fn specified_id(self, id: Option<RojoRef>) -> Self {
        Self {
            specified_id: id,
            ..self
        }
    }

    pub fn middleware(self, middleware: Middleware) -> Self {
        Self {
            middleware: Some(middleware),
            ..self
        }
    }

    pub fn schema(self, schema: Option<String>) -> Self {
        Self { schema, ..self }
    }
}

impl Default for InstanceMetadata {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceContext {
    /// Offline build/export sessions do not negotiate Studio authority.
    #[doc(hidden)]
    #[serde(skip)]
    pub offline_session: bool,
    /// Inherited project authority. Presence of an empty list ends inheritance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_authority: Option<Arc<crate::SyncAuthority>>,
    /// Transient authority for one physical project binding.
    #[doc(hidden)]
    #[serde(skip)]
    pub rule_owner: Option<Arc<crate::snapshot_middleware::RuleOwner>>,
    /// Filter implicit physically empty directory trees without changing explicit bindings.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore_empty_directories: bool,
    /// Enable read-only directory aggregation for sourcemap sessions only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sourcemap_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_unknown_descendants: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub path_ignore_rules: Arc<Vec<PathIgnoreRule>>,
    pub emit_legacy_scripts: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sync_rules: Vec<SyncRule>,
}

impl InstanceContext {
    pub fn new() -> Self {
        Self {
            offline_session: false,
            sync_authority: None,
            rule_owner: None,
            ignore_empty_directories: false,
            sourcemap_only: false,
            ignore_unknown_descendants: None,
            path_ignore_rules: Arc::new(Vec::new()),
            emit_legacy_scripts: emit_legacy_scripts_default().unwrap(),
            sync_rules: Vec::new(),
        }
    }

    pub fn with_emit_legacy_scripts(emit_legacy_scripts: Option<bool>) -> Self {
        Self {
            emit_legacy_scripts: emit_legacy_scripts
                .or_else(emit_legacy_scripts_default)
                .unwrap(),
            ..Self::new()
        }
    }

    /// Extend ignore rules, retaining the last occurrence of an identical rule.
    pub fn add_path_ignore_rules<I>(&mut self, new_rules: I)
    where
        I: IntoIterator<Item = PathIgnoreRule>,
        I::IntoIter: ExactSizeIterator,
    {
        let new_rules = new_rules.into_iter();

        // If the iterator is empty, we can skip cloning our list of ignore
        // rules and appending to it.
        if new_rules.len() == 0 {
            return;
        }

        let new_rules: Vec<_> = new_rules.collect();
        if self.path_ignore_rules.ends_with(&new_rules) {
            return;
        }

        let rules = Arc::make_mut(&mut self.path_ignore_rules);
        for new_rule in new_rules {
            // Ignore rules use last-match precedence, including negations.
            // Skipping an already present rule would preserve the wrong
            // priority; remove its earlier occurrences before appending it.
            rules.retain(|existing| existing != &new_rule);
            rules.push(new_rule);
        }
    }

    /// Extend the list of syncing rules in the context with the given new rules.
    pub fn add_sync_rules<I>(&mut self, new_rules: I)
    where
        I: IntoIterator<Item = SyncRule>,
    {
        self.sync_rules.extend(new_rules);
    }

    /// Clears all sync rules for this InstanceContext
    pub fn clear_sync_rules(&mut self) {
        self.sync_rules.clear();
    }

    pub fn set_emit_legacy_scripts(&mut self, emit_legacy_scripts: bool) {
        self.emit_legacy_scripts = emit_legacy_scripts;
    }

    /// Returns the middleware specified by the first sync rule that
    /// matches the provided path. This does not handle default syncing rules.
    pub fn get_user_sync_rule(&self, path: &Path) -> Option<&SyncRule> {
        self.sync_rules
            .iter()
            .find(|&rule| rule.middleware.is_some() && rule.matches(path))
    }
}

impl Default for InstanceContext {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathIgnoreRule {
    /// The path that this glob is relative to. Since ignore globs are defined
    /// in project files, this will generally be the folder containing the
    /// project file that defined this glob.
    #[serde(serialize_with = "path_serializer::serialize_absolute")]
    pub base_path: PathBuf,

    /// The actual glob that can be matched against the input path.
    pub glob: IgnorableGlob,
}

impl PathIgnoreRule {
    pub fn matches<P: AsRef<Path>>(&self, path: P) -> bool {
        let path = path.as_ref();

        match path.strip_prefix(&self.base_path) {
            Ok(suffix) => self.glob.is_match(suffix),
            Err(_) => {
                // An absolute $path may own sources outside the project folder.
                // Keep the same ../ scope as a relative binding; different drives
                // have no relative representation, but wildcard rules still apply.
                let relative = pathdiff::diff_paths(path, &self.base_path);
                self.glob.is_match(relative.as_deref().unwrap_or(path))
            }
        }
    }

    pub fn is_negation(&self) -> bool {
        self.glob.is_negation()
    }
}

/// Evaluates an ordered list of [`PathIgnoreRule`]s against a path using
/// gitignore-style "last match wins" semantics: a path is ignored if the last
/// rule whose pattern matches it is non-negated. Paths matched by no rule are
/// not ignored.
pub fn is_path_ignored<P: AsRef<Path>>(rules: &[PathIgnoreRule], path: P) -> bool {
    let path = path.as_ref();
    let mut ignored = false;
    for rule in rules {
        if rule.matches(path) {
            ignored = !rule.is_negation();
        }
    }
    ignored
}

/// Represents where a particular Instance or InstanceSnapshot came from.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum InstigatingSource {
    /// The path the Instance was made from.
    Path(#[serde(serialize_with = "path_serializer::serialize_absolute")] PathBuf),
    /// The node in a Project that the Instance was made from.
    ProjectNode {
        #[serde(serialize_with = "path_serializer::serialize_absolute")]
        path: PathBuf,
        name: String,
        node: Box<ProjectNode>,
        parent_class: Option<String>,
    },
}

impl InstigatingSource {
    pub fn path(&self) -> &Path {
        match self {
            Self::Path(path) => path.as_path(),
            Self::ProjectNode { path, .. } => path.as_path(),
        }
    }
}

impl fmt::Debug for InstigatingSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstigatingSource::Path(path) => write!(formatter, "Path({})", path.display()),
            InstigatingSource::ProjectNode {
                name,
                node,
                path,
                parent_class,
            } => write!(
                formatter,
                "ProjectNode({}: {:?}) from path {} and parent class {:?}",
                name,
                node,
                path.display(),
                parent_class,
            ),
        }
    }
}

impl From<PathBuf> for InstigatingSource {
    fn from(path: PathBuf) -> Self {
        InstigatingSource::Path(path)
    }
}

impl From<&Path> for InstigatingSource {
    fn from(path: &Path) -> Self {
        InstigatingSource::Path(path.to_path_buf())
    }
}

/// Represents an user-specified rule for transforming files
/// into Instances using a given middleware.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(try_from = "UncheckedSyncRule")]
pub struct SyncRule {
    /// A pattern used to determine if a file is included in this SyncRule
    #[serde(rename = "pattern")]
    pub include: Glob,
    /// A pattern used to determine if a file is excluded from this SyncRule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Glob>,
    /// The middleware specified by the user for this SyncRule
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub middleware: Option<Middleware>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Arc<crate::snapshot_middleware::RuleDefaults>>,
    /// A suffix to trim off of file names, including the file extension.
    /// If not specified, the file extension is the only thing cut off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// The 'base' of the glob above, allowing it to be used
    /// relative to a path instead of absolute.
    #[serde(skip)]
    pub base_path: PathBuf,
}

#[derive(Deserialize)]
struct UncheckedSyncRule {
    pattern: Glob,
    exclude: Option<Glob>,
    #[serde(rename = "use")]
    middleware: Option<Middleware>,
    suffix: Option<String>,
    defaults: Option<Arc<crate::snapshot_middleware::RuleDefaults>>,
}

impl TryFrom<UncheckedSyncRule> for SyncRule {
    type Error = anyhow::Error;

    fn try_from(rule: UncheckedSyncRule) -> Result<Self, Self::Error> {
        anyhow::ensure!(
            rule.middleware.is_some() || rule.defaults.as_ref().is_some_and(|d| !d.is_empty()),
            "syncRules entry requires use or nonempty defaults"
        );
        anyhow::ensure!(
            rule.middleware.is_some() || rule.suffix.is_none(),
            "defaults-only syncRules entry cannot specify suffix"
        );
        if let Some(defaults) = &rule.defaults {
            defaults.validate()?;
        }
        Ok(Self {
            include: rule.pattern,
            exclude: rule.exclude,
            middleware: rule.middleware,
            defaults: rule.defaults,
            suffix: rule.suffix,
            base_path: PathBuf::new(),
        })
    }
}

impl SyncRule {
    /// Returns whether the given path matches this rule.
    pub fn matches(&self, path: &Path) -> bool {
        match path.strip_prefix(&self.base_path) {
            Ok(suffix) => {
                if let Some(pattern) = &self.exclude {
                    if pattern.is_match(suffix) {
                        return false;
                    }
                }
                self.include.is_match(suffix)
            }
            Err(_) => false,
        }
    }

    pub fn file_name_for_path<'a>(&self, path: &'a Path) -> anyhow::Result<&'a str> {
        if let Some(suffix) = &self.suffix {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .with_context(|| format!("file name of {} is invalid", path.display()))?;
            if file_name.ends_with(suffix) {
                let end = file_name.len().saturating_sub(suffix.len());
                Ok(&file_name[..end])
            } else {
                Ok(file_name)
            }
        } else {
            // If the user doesn't specify a suffix, we assume they just want
            // the name of the file (the file_stem)
            path.file_stem()
                .and_then(|s| s.to_str())
                .with_context(|| format!("file name of {} is invalid", path.display()))
        }
    }
}

#[cfg(test)]
mod ignore_rule_tests {
    use super::*;
    use crate::glob::IgnorableGlob;

    fn rule(base: &str, pattern: &str) -> PathIgnoreRule {
        PathIgnoreRule {
            base_path: base.into(),
            glob: IgnorableGlob::new(pattern).unwrap(),
        }
    }

    #[test]
    fn repeated_ignore_blocks_do_not_accumulate_or_change_cloned_contexts() {
        let block = vec![rule("/project", "**/*.lua"), rule("/project", "!keep.lua")];
        let mut original = InstanceContext::new();
        original.add_path_ignore_rules(block.clone());
        let mut child = original.clone();
        for _ in 0..1000 {
            child.add_path_ignore_rules(block.clone());
        }
        assert_eq!(child.path_ignore_rules.len(), block.len());
        assert_eq!(*child.path_ignore_rules, block);
        assert_eq!(*original.path_ignore_rules, block);
        assert!(is_path_ignored(
            &child.path_ignore_rules,
            "/project/drop.lua"
        ));
        assert!(!is_path_ignored(
            &child.path_ignore_rules,
            "/project/keep.lua"
        ));
    }

    #[test]
    fn duplicate_ignore_rules_retain_the_last_match_precedence_and_base_scope() {
        let mut context = InstanceContext::new();
        let positive = rule("/project", "**/*.lua");
        let negative = rule("/project", "!keep.lua");
        let other_base = rule("/elsewhere", "**/*.lua");
        context.add_path_ignore_rules(vec![positive.clone(), negative.clone(), other_base.clone()]);
        context.add_path_ignore_rules(vec![positive.clone(), positive.clone()]);
        assert_eq!(context.path_ignore_rules.len(), 3);
        assert_eq!(
            *context.path_ignore_rules,
            vec![negative.clone(), other_base.clone(), positive.clone()]
        );
        assert!(is_path_ignored(
            &context.path_ignore_rules,
            "/project/keep.lua"
        ));
        assert!(is_path_ignored(
            &context.path_ignore_rules,
            "/elsewhere/keep.lua"
        ));
        context.add_path_ignore_rules(vec![negative]);
        assert!(!is_path_ignored(
            &context.path_ignore_rules,
            "/project/keep.lua"
        ));
        assert!(is_path_ignored(
            &context.path_ignore_rules,
            "/elsewhere/keep.lua"
        ));
        assert_eq!(context.path_ignore_rules.len(), 3);
    }

    #[test]
    fn resnapshotting_a_project_with_its_saved_context_does_not_grow_ignore_rules() {
        use crate::snapshot_middleware::snapshot_from_vfs;
        use memofs::{InMemoryFs, Vfs, VfsSnapshot};

        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(r#"{"name":"Root","globIgnorePaths":["**/*.ignored.lua"],"tree":{"$path":"src"}}"#)),
            ("src", VfsSnapshot::dir([("main.lua", VfsSnapshot::file("return 1"))])),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        let mut context = InstanceContext::new();
        for _ in 0..100 {
            let snapshot =
                snapshot_from_vfs(&context, &vfs, Path::new("/project/default.project.json"))
                    .unwrap()
                    .unwrap();
            context = snapshot.metadata.context;
            assert_eq!(context.path_ignore_rules.len(), 1);
            assert_eq!(snapshot.children.len(), 1);
        }
    }
}
