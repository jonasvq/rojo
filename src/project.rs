use std::{
    collections::HashSet,
    ffi::OsStr,
    io,
    net::IpAddr,
    path::{Path, PathBuf},
};

use memofs::{IoResultExt as _, Vfs};
use serde::{Deserialize, Serialize};
#[cfg(feature = "cli")]
use std::collections::BTreeMap;
use thiserror::Error;

use crate::{glob::IgnorableGlob, json, snapshot::SyncRule, syncback::SyncbackRules};

pub use rojo_project_types::{
    ClassAuthorityPolicy, MultiplePaths, OptionalPathNode, PathNode, ProjectFormatVersion,
    ProjectNode, PropertyAuthority, RuleDefaultMask, StructureAuthority, SyncAuthority,
};

/// Represents 'default' project names that act as `init` files
pub static DEFAULT_PROJECT_NAMES: [&str; 2] = ["default.project.json", "default.project.jsonc"];

/// Directory-entry aliases, active only with an explicit V2 declaration.
pub static INIT_PROJECT_NAMES: [&str; 2] = ["init.project.json", "init.project.jsonc"];

fn deserialize_format_version<'de, D>(
    deserializer: D,
) -> Result<Option<ProjectFormatVersion>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Only omission means legacy defaults; an explicit null is not a version.
    ProjectFormatVersion::deserialize(deserializer)
        .map(Some)
        .map_err(|error| serde::de::Error::custom(format!("Invalid project $version: {error}")))
}

/// Error type returned by any function that handles projects.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ProjectError(#[from] Error);

#[derive(Debug, Error)]
enum Error {
    #[error("Ambiguous project directory entries: {paths}. Specify a project file explicitly or remove the conflicting entry.")]
    AmbiguousEntries { paths: String },
    #[error(
        "Rojo requires a project file, but no project file was found in path {}\n\
        See https://rojo.space/docs/ for guides and documentation.",
        .path.display()
    )]
    NoProjectFound { path: PathBuf },

    #[error("The folder for the provided project cannot be used as a project name: {}\n\
            Consider setting the `name` field on this project.", .path.display())]
    FolderNameInvalid { path: PathBuf },

    #[error("The file name of the provided project cannot be used as a project name: {}.\n\
            Consider setting the `name` field on this project.", .path.display())]
    ProjectNameInvalid { path: PathBuf },

    #[error(transparent)]
    Io {
        #[from]
        source: io::Error,
    },

    #[error("Error parsing Rojo project in path {}", .path.display())]
    Json {
        source: serde_json::Error,
        path: PathBuf,
    },
}

/// Contains all of the configuration for a Rojo-managed project.
///
/// Project files are stored in `.project.json` files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Project {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<String>,

    /// Opt into versioned defaults without migrating source files.
    #[serde(
        rename = "$version",
        default,
        deserialize_with = "deserialize_format_version",
        skip_serializing_if = "Option::is_none"
    )]
    pub format_version: Option<ProjectFormatVersion>,

    /// The name of the top-level instance described by the project.
    pub name: Option<String>,

    /// The tree of instances described by this project. Projects always
    /// describe at least one instance.
    pub tree: ProjectNode,

    /// If specified, sets the default port that `rojo serve` should use when
    /// using this project for live sync.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serve_port: Option<u16>,

    /// If specified, contains the set of place IDs that this project is
    /// compatible with when doing live sync.
    ///
    /// This setting is intended to help prevent syncing a Rojo project into the
    /// wrong Roblox place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serve_place_ids: Option<HashSet<u64>>,

    /// If specified, contains a set of place IDs that this project is
    /// not compatible with when doing live sync.
    ///
    /// This setting is intended to help prevent syncing a Rojo project into the
    /// wrong Roblox place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_place_ids: Option<HashSet<u64>>,

    /// If specified, sets the current place's place ID when connecting to the
    /// Rojo server from Roblox Studio.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_id: Option<u64>,

    /// If specified, sets the current place's game ID when connecting to the
    /// Rojo server from Roblox Studio.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<u64>,

    /// If specified, this address will be used in place of the default address
    /// As long as --address is unprovided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serve_address: Option<IpAddr>,

    /// Additional `Host`/`Origin` header values that `rojo serve` will accept
    /// beyond `localhost` and the bind address, such as a hostname like
    /// `mypc.lan` used to reach a network-exposed server by name. Listing any
    /// host also turns on `Host`/`Origin` validation for binds where it is
    /// otherwise off (such as `0.0.0.0`). The `--allowed-hosts` CLI option
    /// overrides this field when provided.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub serve_allowed_hosts: Vec<String>,

    /// Determines if Rojo should emit scripts with the appropriate `RunContext`
    /// for `*.client.lua` and `*.server.lua` files in the project instead of
    /// using `Script` and `LocalScript` Instances.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emit_legacy_scripts: Option<bool>,

    /// Omit implicit physically empty directory trees; nested projects inherit omission.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_empty_directories: Option<bool>,

    /// A list of globs, relative to the folder the project file is in, that
    /// match files that should be excluded if Rojo encounters them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glob_ignore_paths: Vec<IgnorableGlob>,

    /// A list of rules for syncback with this project file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub syncback_rules: Option<SyncbackRules>,

    /// A list of mappings of globs to syncing rules. If a file matches a glob,
    /// it will be 'transformed' into an Instance following the rule provided.
    /// Globs are relative to the folder the project file is in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sync_rules: Vec<SyncRule>,

    /// Replace inherited managed-write policies. An empty list restores legacy
    /// authority; omission inherits the enclosing project's list.
    #[serde(
        default,
        deserialize_with = "deserialize_sync_authority",
        skip_serializing_if = "Option::is_none"
    )]
    pub sync_authority: Option<SyncAuthority>,

    /// The path to the file that this project came from. Relative paths in the
    /// project should be considered relative to the parent of this field, also
    /// given by `Project::folder_location`.
    #[serde(skip)]
    pub file_location: PathBuf,
}

fn deserialize_sync_authority<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SyncAuthority>, D::Error> {
    SyncAuthority::deserialize(deserializer).map(Some)
}

impl Project {
    pub(crate) fn effective_emit_legacy_scripts(&self) -> bool {
        self.emit_legacy_scripts.unwrap_or_else(|| {
            self.format_version != Some(ProjectFormatVersion::V2)
                && crate::snapshot_middleware::emit_legacy_scripts_default().unwrap()
        })
    }

    /// Tells whether the given path describes a Rojo project.
    pub fn is_project_file(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".project.json") || name.ends_with(".project.jsonc"))
            .unwrap_or(false)
    }

    /// Attempt to locate a project represented by the given path.
    ///
    /// This will find a project if the path refers to a `.project.json` file,
    /// or is a folder with a default project or explicit V2 init entry.
    fn locate(vfs: &Vfs, path: &Path) -> Result<Option<PathBuf>, ProjectError> {
        match vfs.metadata(path).with_not_found().map_err(Error::from)? {
            Some(meta) if meta.is_dir() => Self::directory_entry(vfs, path),
            Some(meta) if meta.is_file() && Self::is_project_file(path) => {
                Ok(Some(path.to_owned()))
            }
            _ => Ok(None),
        }
    }

    pub(crate) fn directory_entry(vfs: &Vfs, path: &Path) -> Result<Option<PathBuf>, ProjectError> {
        let exists = |candidate: &Path| {
            vfs.metadata(candidate)
                .with_not_found()
                .map(|meta| meta.is_some())
                .map_err(|error| Error::Io {
                    source: io::Error::new(
                        error.kind(),
                        format!("Could not inspect {}: {error}", candidate.display()),
                    ),
                })
        };
        let mut defaults = Vec::new();
        for name in DEFAULT_PROJECT_NAMES {
            let candidate = path.join(name);
            if exists(&candidate)? {
                defaults.push(candidate);
            }
        }
        let mut aliases = Vec::new();
        for name in INIT_PROJECT_NAMES {
            let candidate = path.join(name);
            if exists(&candidate)?
                && Self::load_exact(vfs, &candidate, Some("init"))?.format_version
                    == Some(ProjectFormatVersion::V2)
            {
                aliases.push(candidate);
            }
        }
        if !aliases.is_empty() {
            let mut conflicts = aliases.clone();
            conflicts.extend(defaults);
            for (_, name) in crate::snapshot_middleware::INIT_PATHS {
                let candidate = path.join(name);
                if exists(&candidate)? {
                    conflicts.push(candidate);
                }
            }
            if conflicts.len() > 1 {
                return Err(Error::AmbiguousEntries {
                    paths: conflicts
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                }
                .into());
            }
            Ok(aliases.pop())
        } else {
            Ok(defaults.into_iter().next())
        }
    }

    /// Sets the name of a project. The order it handles is as follows:
    ///
    /// - Default entries and V2 init entries use their containing folder's name.
    /// - Otherwise, a supplied fallback wins.
    /// - A directly loaded V1 init project uses its ordinary stem, `init`.
    fn set_file_name(&mut self, fallback: Option<&str>) -> Result<(), Error> {
        let file_name = self
            .file_location
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| Error::ProjectNameInvalid {
                path: self.file_location.clone(),
            })?;

        let init_alias = INIT_PROJECT_NAMES.contains(&file_name);
        if DEFAULT_PROJECT_NAMES.contains(&file_name)
            || (init_alias && self.format_version == Some(ProjectFormatVersion::V2))
        {
            let folder_name = self.folder_location().file_name().and_then(OsStr::to_str);
            if let Some(folder_name) = folder_name {
                self.name = Some(folder_name.to_string());
                return Ok(());
            } else {
                return Err(Error::FolderNameInvalid {
                    path: self.file_location.clone(),
                });
            }
        }
        if let Some(fallback) = fallback {
            self.name = Some(fallback.to_string());
        } else if init_alias {
            self.name = Some("init".to_owned());
        } else {
            // As of the time of writing (July 10, 2024) there is no way for
            // this code path to be reachable. It can in theory be reached from
            // both `load_fuzzy` and `load_exact` but in practice it's never
            // invoked.
            // If you're adding this codepath, make sure a test for it exists
            // and that it handles sync rules appropriately.
            todo!(
                "set_file_name doesn't support loading project files that aren't default.project.json without a fallback provided"
            );
        }

        Ok(())
    }

    /// Loads a Project file from the provided contents with its source set as
    /// the provided location.
    #[cfg(feature = "cli")]
    pub(crate) fn from_stdin_bytes(
        contents: &[u8],
        origin: PathBuf,
        name: &str,
    ) -> Result<Self, ProjectError> {
        Ok(Self::load_from_slice(contents, origin, Some(name))?)
    }

    fn load_from_slice(
        contents: &[u8],
        project_file_location: PathBuf,
        fallback_name: Option<&str>,
    ) -> Result<Self, Error> {
        let mut project: Self = json::from_slice(contents).map_err(|e| Error::Json {
            source: serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{e:#}"),
            )),
            path: project_file_location.clone(),
        })?;
        project.file_location = project_file_location;
        project.check_compatibility();
        if project.name.is_none() {
            project.set_file_name(fallback_name)?;
        }

        Ok(project)
    }

    /// Loads a Project from a path. This will find the project if it refers to
    /// a `.project.json` file or if it refers to a directory that contains a
    /// default project or an explicit V2 init entry. Directory ambiguities are
    /// rejected; explicitly naming a file bypasses directory selection.
    pub fn load_fuzzy(
        vfs: &Vfs,
        fuzzy_project_location: &Path,
    ) -> Result<Option<Self>, ProjectError> {
        if let Some(project_path) = Self::locate(vfs, fuzzy_project_location)? {
            let contents = vfs.read(&project_path).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => Error::NoProjectFound {
                    path: project_path.to_path_buf(),
                },
                _ => e.into(),
            })?;

            Ok(Some(Self::load_from_slice(&contents, project_path, None)?))
        } else {
            Ok(None)
        }
    }

    /// Loads a Project from a path.
    pub fn load_exact(
        vfs: &Vfs,
        project_file_location: &Path,
        fallback_name: Option<&str>,
    ) -> Result<Self, ProjectError> {
        log::debug!(
            "Loading project file from {}",
            project_file_location.display()
        );
        let project_path = project_file_location.to_path_buf();
        let contents = vfs.read(&project_path).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NoProjectFound {
                path: project_path.to_path_buf(),
            },
            _ => e.into(),
        })?;

        Ok(Self::load_from_slice(
            &contents,
            project_path,
            fallback_name,
        )?)
    }

    #[cfg(any(feature = "cli", test))]
    pub(crate) fn load_initial_project(vfs: &Vfs, path: &Path) -> Result<Self, ProjectError> {
        if Self::is_project_file(path) {
            Self::load_exact(vfs, path, None)
        } else {
            if let Some(project_path) = Self::directory_entry(vfs, path)? {
                return Self::load_exact(vfs, &project_path, None);
            }
            Err(Error::NoProjectFound {
                path: path.to_path_buf(),
            }
            .into())
        }
    }

    /// Checks if there are any compatibility issues with this project file and
    /// warns the user if there are any.
    fn check_compatibility(&self) {
        validate_reserved_names(&self.tree);
    }

    #[cfg(feature = "cli")]
    pub(crate) fn warn_duplicate_paths(&self, vfs: &Vfs) {
        fn visit(
            node: &ProjectNode,
            location: String,
            folder: &Path,
            vfs: &Vfs,
            bindings: &mut BTreeMap<PathBuf, Vec<String>>,
        ) {
            if let Some(path) = &node.path {
                // Canonicalization recognizes aliases without guessing whether
                // a missing optional path will eventually refer to the same file.
                for source in path.paths() {
                    if let Ok(path) = vfs.canonicalize(folder.join(source)) {
                        let locations = bindings.entry(path).or_default();
                        if !locations.contains(&location) {
                            locations.push(location.clone());
                        }
                    }
                }
            }
            for (name, child) in &node.children {
                visit(child, format!("{location}/{name}"), folder, vfs, bindings);
            }
        }

        let mut bindings = BTreeMap::new();
        visit(
            &self.tree,
            "tree".into(),
            self.folder_location(),
            vfs,
            &mut bindings,
        );
        for (path, locations) in bindings {
            if locations.len() > 1 {
                log::warn!(
                    "Duplicate $path binding in {}: {} is mapped at {}. \
                     This produces separate instances; check whether these aliases are intentional.",
                    self.file_location.display(), path.display(), locations.join(", ")
                );
            }
        }
    }

    pub fn folder_location(&self) -> &Path {
        self.file_location.parent().unwrap()
    }
}

fn validate_reserved_names(node: &ProjectNode) {
    for (name, child) in &node.children {
        if name.starts_with('$') {
            log::warn!(
                "Keys starting with '$' are reserved by Rojo to ensure forward compatibility."
            );
            log::warn!(
                "This project uses the key '{}', which should be renamed.",
                name
            );
        }
        validate_reserved_names(child);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn path_node_required() {
        let path_node: PathNode = json::from_str(r#""src""#).unwrap();
        assert_eq!(path_node, PathNode::Required(PathBuf::from("src")));
    }

    #[test]
    fn path_node_optional() {
        let path_node: PathNode = json::from_str(r#"{ "optional": "src" }"#).unwrap();
        assert_eq!(
            path_node,
            PathNode::Optional(OptionalPathNode::new(PathBuf::from("src")))
        );
    }

    #[test]
    fn project_node_required() {
        let project_node: ProjectNode = json::from_str(
            r#"{
                "$path": "src"
            }"#,
        )
        .unwrap();

        assert_eq!(
            project_node.path,
            Some(PathNode::Required(PathBuf::from("src")))
        );
    }

    #[test]
    fn project_node_optional() {
        let project_node: ProjectNode = json::from_str(
            r#"{
                "$path": { "optional": "src" }
            }"#,
        )
        .unwrap();

        assert_eq!(
            project_node.path,
            Some(PathNode::Optional(OptionalPathNode::new(PathBuf::from(
                "src"
            ))))
        );
    }

    #[test]
    fn project_node_none() {
        let project_node: ProjectNode = json::from_str(
            r#"{
                "$className": "Folder"
            }"#,
        )
        .unwrap();

        assert_eq!(project_node.path, None);
    }

    #[test]
    fn project_node_optional_serialize_absolute() {
        let project_node: ProjectNode = json::from_str(
            r#"{
                "$path": { "optional": "..\\src" }
            }"#,
        )
        .unwrap();

        let serialized = serde_json::to_string(&project_node).unwrap();
        assert_eq!(serialized, r#"{"$path":{"optional":"../src"}}"#);
    }

    #[test]
    fn project_node_optional_serialize_absolute_no_change() {
        let project_node: ProjectNode = json::from_str(
            r#"{
                "$path": { "optional": "../src" }
            }"#,
        )
        .unwrap();

        let serialized = serde_json::to_string(&project_node).unwrap();
        assert_eq!(serialized, r#"{"$path":{"optional":"../src"}}"#);
    }

    #[test]
    fn project_node_optional_serialize_optional() {
        let project_node: ProjectNode = json::from_str(
            r#"{
                "$path": "..\\src"
            }"#,
        )
        .unwrap();

        let serialized = serde_json::to_string(&project_node).unwrap();
        assert_eq!(serialized, r#"{"$path":"../src"}"#);
    }

    #[test]
    fn project_with_jsonc_features() {
        // Test that JSONC features (comments and trailing commas) are properly handled
        let project_json = r#"{
            // This is a single-line comment
            "name": "TestProject",
            /* This is a
               multi-line comment */
            "tree": {
                "$path": "src", // Comment after value
            },
            "servePort": 34567,
            "emitLegacyScripts": false,
            // Test glob parsing with comments
            "globIgnorePaths": [
                "**/*.spec.lua", // Ignore test files
                "**/*.test.lua",
            ],
            "syncRules": [
                {
                    "pattern": "*.data.json",
                    "use": "json", // Trailing comma in object
                },
                {
                    "pattern": "*.module.lua",
                    "use": "moduleScript",
                }, // Trailing comma in array
            ], // Another trailing comma
        }"#;

        let project = Project::load_from_slice(
            project_json.as_bytes(),
            PathBuf::from("/test/default.project.jsonc"),
            None,
        )
        .expect("Failed to parse project with JSONC features");

        // Verify the parsed values
        assert_eq!(project.name, Some("TestProject".to_string()));
        assert_eq!(project.serve_port, Some(34567));
        assert_eq!(project.emit_legacy_scripts, Some(false));

        // Verify glob_ignore_paths were parsed correctly
        assert_eq!(project.glob_ignore_paths.len(), 2);
        assert!(project.glob_ignore_paths[0].is_match("test/foo.spec.lua"));
        assert!(project.glob_ignore_paths[1].is_match("test/bar.test.lua"));

        // Verify sync_rules were parsed correctly
        assert_eq!(project.sync_rules.len(), 2);
        assert!(project.sync_rules[0].include.is_match("data.data.json"));
        assert!(project.sync_rules[1].include.is_match("init.module.lua"));
    }

    #[test]
    fn project_with_serve_allowed_hosts() {
        let project_json = r#"{
            "name": "TestProject",
            "tree": { "$path": "src" },
            "serveAllowedHosts": ["mypc.lan", "192.168.1.5"]
        }"#;

        let project = Project::load_from_slice(
            project_json.as_bytes(),
            PathBuf::from("/test/default.project.json"),
            None,
        )
        .expect("Failed to parse project with serveAllowedHosts");

        assert_eq!(project.serve_allowed_hosts, vec!["mypc.lan", "192.168.1.5"]);
    }

    #[test]
    fn project_without_serve_allowed_hosts_defaults_to_empty() {
        let project_json = r#"{
            "name": "TestProject",
            "tree": { "$path": "src" }
        }"#;

        let project = Project::load_from_slice(
            project_json.as_bytes(),
            PathBuf::from("/test/default.project.json"),
            None,
        )
        .expect("Failed to parse project");

        assert!(project.serve_allowed_hosts.is_empty());
    }

    #[test]
    fn glob_ignore_paths_negation() {
        let project_json = r#"{
            "name": "TestProject",
            "tree": { "$path": "src" },
            "globIgnorePaths": [
                "**/*.spec.lua",
                "!keep.spec.lua",
                "\\!literal.lua"
            ]
        }"#;

        let project = Project::load_from_slice(
            project_json.as_bytes(),
            PathBuf::from("/test/default.project.json"),
            None,
        )
        .expect("project should parse");

        let paths = &project.glob_ignore_paths;
        assert_eq!(paths.len(), 3);

        assert!(!paths[0].is_negation());
        assert!(paths[0].is_match("foo.spec.lua"));

        assert!(paths[1].is_negation());
        assert!(paths[1].is_match("keep.spec.lua"));

        // `\!literal.lua` should match a file literally named `!literal.lua`,
        // not be parsed as a negation.
        assert!(!paths[2].is_negation());
        assert!(paths[2].is_match("!literal.lua"));

        let rules: Vec<_> = paths
            .iter()
            .map(|g| crate::snapshot::PathIgnoreRule {
                base_path: PathBuf::from("/test"),
                glob: g.clone(),
            })
            .collect();
        assert!(crate::snapshot::is_path_ignored(
            &rules,
            "/test/foo.spec.lua"
        ));
        assert!(!crate::snapshot::is_path_ignored(
            &rules,
            "/test/keep.spec.lua"
        ));
        assert!(!crate::snapshot::is_path_ignored(&rules, "/test/plain.lua"));
    }
}
