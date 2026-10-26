// Recursion limit bump is to support Ritz, a JSX-like proc macro used for
// Rojo's web UI currently.
#![recursion_limit = "1024"]

#[cfg(feature = "cli")]
pub mod cli;

#[cfg(test)]
mod tree_view;

mod binary_serialization;
mod builder;
#[cfg(feature = "cli")]
mod change_processor;
#[cfg(any(feature = "cli", test))]
mod diagnostic_context;
mod glob;
mod json;
mod lua_ast;
#[cfg(feature = "cli")]
mod message_queue;
mod multimap;
mod path_serializer;
mod project;
mod resolution;
mod rojo_ref;
#[cfg(feature = "cli")]
mod serve_session;
mod session_id;
mod snapshot;
mod snapshot_middleware;
#[cfg(feature = "cli")]
mod sync_authority;
mod syncback;
mod variant_eq;
#[cfg(feature = "cli")]
mod web;
#[cfg(feature = "cli")]
mod write_receipts;

#[path = "web/interface.rs"]
pub mod web_api;

// TODO: Work out what we should expose publicly

pub use builder::{build_project, write_project, BuildFormat};
pub use project::*;
pub use resolution::{AmbiguousValue, UnresolvedValue};
pub use rojo_ref::*;
pub use session_id::SessionId;
pub use snapshot::{
    InstanceContext, InstanceMetadata, InstanceSnapshot, InstanceWithMeta, InstanceWithMetaMut,
    RojoDescendants, RojoTree, SyncRule,
};
pub use snapshot_middleware::{snapshot_from_vfs, Middleware, RuleDefaults, ScriptType};
pub use syncback::{syncback_loop, FsSnapshot, SyncbackData, SyncbackSnapshot};
