//! Defines all the structs needed to interact with the Rojo Serve API. This is
//! useful for tests to be able to use the same data structures as the
//! implementation.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use rbx_dom_weak::{
    types::{Ref, Variant},
    Ustr, UstrMap,
};
use serde::{Deserialize, Serialize};
use strum::Display;

use crate::session_id::SessionId;
#[cfg(any(feature = "cli", test))]
use crate::snapshot::InstanceMetadata as RojoInstanceMetadata;
#[cfg(feature = "cli")]
use crate::snapshot::InstanceWithMeta;
#[cfg(any(feature = "cli", test))]
use {
    crate::snapshot::{AppliedPatchInstance, AppliedPatchSet},
    rbx_dom_weak::types::VariantType,
};

/// Server version to report over the API, not exposed outside this crate.
#[cfg(feature = "cli")]
pub(crate) const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Current protocol version, which is required to match.
pub const PROTOCOL_VERSION: u64 = 5;

/// Message returned by Rojo API when a change has occurred.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeMessage<'a> {
    pub removed: Vec<Ref>,
    pub added: HashMap<Ref, Instance<'a>>,
    pub updated: Vec<InstanceUpdate>,
}

#[cfg(any(feature = "cli", test))]
impl<'a> SubscribeMessage<'a> {
    pub(crate) fn from_patch_update(patch: &'a AppliedPatchSet) -> Self {
        let removed = patch.removed.clone();
        let added = patch
            .added_instances
            .iter()
            .map(|(&id, instance)| (id, Instance::from_applied_instance(id, instance)))
            .collect();

        let updated = patch
            .updated
            .iter()
            .map(|update| {
                let changed_metadata = update
                    .changed_metadata
                    .as_ref()
                    .map(InstanceMetadata::from_rojo_metadata);

                let changed_properties = update
                    .changed_properties
                    .iter()
                    .filter(|(_key, value)| property_filter(value.as_ref()))
                    .map(|(&key, value)| (key, value.clone()))
                    .collect();

                InstanceUpdate {
                    id: update.id,
                    changed_name: update.changed_name.clone(),
                    changed_class_name: update.changed_class_name,
                    changed_properties,
                    changed_metadata,
                }
            })
            .collect();

        Self {
            removed,
            added,
            updated,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceUpdate {
    pub id: Ref,
    pub changed_name: Option<String>,
    pub changed_class_name: Option<Ustr>,

    // TODO: Transform from UstrMap<String, Option<_>> to something else, since
    // null will get lost when decoding from JSON in some languages.
    #[serde(default)]
    pub changed_properties: UstrMap<Option<Variant>>,
    pub changed_metadata: Option<InstanceMetadata>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceMetadata {
    pub ignore_unknown_instances: bool,
}

#[cfg(any(feature = "cli", test))]
impl InstanceMetadata {
    pub(crate) fn from_rojo_metadata(meta: &RojoInstanceMetadata) -> Self {
        Self {
            ignore_unknown_instances: meta.ignore_unknown_instances,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Instance<'a> {
    pub id: Ref,
    pub parent: Ref,
    pub name: Cow<'a, str>,
    pub class_name: Ustr,
    pub properties: UstrMap<Cow<'a, Variant>>,
    pub children: Cow<'a, [Ref]>,
    pub metadata: Option<InstanceMetadata>,
}

impl Instance<'_> {
    #[cfg(any(feature = "cli", test))]
    fn from_applied_instance(id: Ref, source: &AppliedPatchInstance) -> Instance<'_> {
        Instance {
            id,
            parent: source.parent,
            name: Cow::Borrowed(&source.name),
            class_name: source.class_name,
            properties: source
                .properties
                .iter()
                .filter(|(_key, value)| property_filter(Some(value)))
                .map(|(&key, value)| (key, Cow::Borrowed(value)))
                .collect(),
            children: Cow::Borrowed(&source.children),
            metadata: Some(InstanceMetadata {
                ignore_unknown_instances: source.ignore_unknown_instances,
            }),
        }
    }

    #[cfg(feature = "cli")]
    pub(crate) fn from_rojo_instance(source: InstanceWithMeta<'_>) -> Instance<'_> {
        let properties = source
            .properties()
            .iter()
            .filter(|(_key, value)| property_filter(Some(value)))
            .map(|(key, value)| (*key, Cow::Borrowed(value)))
            .collect();

        Instance {
            id: source.id(),
            parent: source.parent(),
            name: Cow::Borrowed(source.name()),
            class_name: source.class_name(),
            properties,
            children: Cow::Borrowed(source.children()),
            metadata: Some(InstanceMetadata::from_rojo_metadata(source.metadata())),
        }
    }
}

#[cfg(any(feature = "cli", test))]
fn property_filter(value: Option<&Variant>) -> bool {
    let ty = value.map(|value| value.ty());

    // Lua can't do anything with SharedString values. They also can't be
    // serialized directly by Serde!
    ty != Some(VariantType::SharedString)
}

/// Response body from /api/rojo
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfoResponse {
    pub session_id: SessionId,
    pub server_version: String,
    pub protocol_version: u64,
    pub project_name: String,
    pub expected_place_ids: Option<HashSet<u64>>,
    pub unexpected_place_ids: Option<HashSet<u64>>,
    pub game_id: Option<u64>,
    pub place_id: Option<u64>,
    pub root_instance_id: Ref,
}

/// Response body from /api/read/{id}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResponse<'a> {
    pub session_id: SessionId,
    pub message_cursor: u32,
    pub instances: HashMap<Ref, Instance<'a>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteRequest {
    pub session_id: SessionId,
    pub removed: Vec<Ref>,

    #[serde(default)]
    pub added: HashMap<Ref, ()>,
    pub updated: Vec<InstanceUpdate>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResponse {
    pub session_id: SessionId,
}

/// Packet type enum for different websocket message types
#[derive(Debug, Serialize, Deserialize, Display, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum SocketPacketType {
    Messages,
    // TODO: Can we cleanly use the socket for all communication?
    // Serialize,
    // RefPatch,
}

/// Body content for messages packet type
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesPacket<'a> {
    pub message_cursor: u32,
    pub messages: Vec<SubscribeMessage<'a>>,
}

/// Body content for different packet types
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SocketPacketBody<'a> {
    Messages(MessagesPacket<'a>),
    // TODO: Can we cleanly use the socket for all communication?
    // Serialize(SerializePacket),
    // RefPatch(RefPatchPacket<'a>),
}

/// Message content from /api/socket
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocketPacket<'a> {
    pub session_id: SessionId,
    pub packet_type: SocketPacketType,
    pub body: SocketPacketBody<'a>,
}

/// Response body from /api/open/{id}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResponse {
    pub session_id: SessionId,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerializeRequest {
    pub session_id: SessionId,
    pub ids: Vec<Ref>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerializeResponse {
    pub session_id: SessionId,
    #[serde(with = "serde_bytes")]
    pub model_contents: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefPatchRequest {
    pub session_id: SessionId,
    pub ids: HashSet<Ref>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefPatchResponse<'a> {
    pub session_id: SessionId,
    pub patch: SubscribeMessage<'a>,
}

/// General response type returned from all Rojo routes
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    kind: ErrorResponseKind,
    details: String,
}

impl ErrorResponse {
    pub fn not_found<S: Into<String>>(details: S) -> Self {
        Self {
            kind: ErrorResponseKind::NotFound,
            details: details.into(),
        }
    }

    pub fn bad_request<S: Into<String>>(details: S) -> Self {
        Self {
            kind: ErrorResponseKind::BadRequest,
            details: details.into(),
        }
    }

    pub fn forbidden<S: Into<String>>(details: S) -> Self {
        Self {
            kind: ErrorResponseKind::Forbidden,
            details: details.into(),
        }
    }

    pub fn internal_error<S: Into<String>>(details: S) -> Self {
        Self {
            kind: ErrorResponseKind::InternalError,
            details: details.into(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ErrorResponseKind {
    NotFound,
    BadRequest,
    Forbidden,
    InternalError,
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::snapshot::{
        apply_patch_set, InstanceSnapshot, PatchAdd, PatchSet, PatchUpdate, RojoTree,
    };
    use rbx_dom_weak::ustr;

    fn add(tree: &mut RojoTree, parent_id: Ref, instance: InstanceSnapshot) -> AppliedPatchSet {
        apply_patch_set(
            tree,
            PatchSet {
                added_instances: vec![PatchAdd {
                    parent_id,
                    instance,
                }],
                ..PatchSet::new()
            },
        )
    }

    #[test]
    fn delayed_addition_survives_subtree_deletion() {
        let mut tree = RojoTree::new(InstanceSnapshot::new().name("Root").class_name("Folder"));
        let root = tree.get_root_id();
        let first = add(
            &mut tree,
            root,
            InstanceSnapshot::new()
                .name("Folder")
                .class_name("Folder")
                .children(vec![InstanceSnapshot::new()
                    .name("Child")
                    .class_name("ModuleScript")
                    .property("Source", "return 1")]),
        );
        let folder = first.added[0];
        let child = tree.get_instance(folder).unwrap().children()[0];
        apply_patch_set(
            &mut tree,
            PatchSet {
                removed_instances: vec![folder],
                ..PatchSet::new()
            },
        );
        let message = SubscribeMessage::from_patch_update(&first);
        assert_eq!(message.added.len(), 2);
        assert_eq!(message.added[&folder].parent, root);
        assert_eq!(message.added[&child].parent, folder);
        assert_eq!(message.added[&child].name, "Child");
    }

    #[test]
    fn delayed_addition_keeps_its_original_source() {
        let mut tree = RojoTree::new(InstanceSnapshot::new().name("Root").class_name("Folder"));
        let root = tree.get_root_id();
        let first = add(
            &mut tree,
            root,
            InstanceSnapshot::new()
                .name("Script")
                .class_name("ModuleScript")
                .property("Source", "return 1"),
        );
        let id = first.added[0];
        apply_patch_set(
            &mut tree,
            PatchSet {
                updated_instances: vec![PatchUpdate {
                    id,
                    changed_name: None,
                    changed_class_name: None,
                    changed_metadata: None,
                    changed_properties: [(
                        ustr("Source"),
                        Some(Variant::String("return 2".into())),
                    )]
                    .into_iter()
                    .collect(),
                }],
                ..PatchSet::new()
            },
        );
        let message = SubscribeMessage::from_patch_update(&first);
        assert_eq!(
            message.added[&id].properties[&ustr("Source")].as_ref(),
            &Variant::String("return 1".into())
        );
    }

    #[test]
    fn delayed_addition_excludes_children_created_in_later_patches() {
        let mut tree = RojoTree::new(InstanceSnapshot::new().name("Root").class_name("Folder"));
        let root = tree.get_root_id();
        let first = add(
            &mut tree,
            root,
            InstanceSnapshot::new().name("Folder").class_name("Folder"),
        );
        let folder = first.added[0];
        add(
            &mut tree,
            folder,
            InstanceSnapshot::new().name("Later").class_name("Folder"),
        );
        let message = SubscribeMessage::from_patch_update(&first);
        assert_eq!(message.added.len(), 1);
        assert!(message.added[&folder].children.is_empty());
    }
}
