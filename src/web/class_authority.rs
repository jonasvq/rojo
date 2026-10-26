//! Negotiated, read-only class policy transport. No mutation grants.
use std::collections::HashMap;

use rbx_dom_weak::types::Ref;

use crate::{snapshot::RojoTree, PropertyAuthority};

pub(super) const CAPABILITY: &str = "classAuthorityV1";

pub(super) fn negotiated(query: Option<&str>) -> bool {
    query.is_some_and(|query| query.split('&').any(|part| part == "classAuthorityV1=true"))
}

pub(super) fn label(policy: Option<PropertyAuthority>) -> &'static str {
    match policy {
        None => "legacy",
        Some(PropertyAuthority::Filesystem) => "filesystem",
        Some(PropertyAuthority::Studio) => "studio",
        Some(PropertyAuthority::TwoWay) => "twoWay",
    }
}

pub(super) fn read(
    tree: &RojoTree,
    ids: impl Iterator<Item = Ref>,
) -> anyhow::Result<HashMap<Ref, &'static str>> {
    let mut modes = HashMap::new();
    for id in ids {
        if let Some(node) = tree.get_instance(id) {
            let policy = node
                .metadata()
                .effective_sync_authority
                .as_ref()
                .filter(|authority| !authority.policies().is_empty())
                .map(|authority| authority.resolve(node.class_name().as_str()))
                .transpose()?
                .flatten()
                .map(|policy| policy.properties);
            modes.insert(id, label(policy));
        }
    }
    Ok(modes)
}

pub(super) fn requires_client(tree: &RojoTree) -> bool {
    tree.descendants(tree.get_root_id()).any(|node| {
        node.metadata()
            .effective_sync_authority
            .as_ref()
            .is_some_and(|authority| {
                authority
                    .policies()
                    .iter()
                    .any(|policy| policy.properties != PropertyAuthority::Filesystem)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        snapshot::{
            apply_patch_set, InstanceMetadata, InstanceSnapshot, PatchAdd, PatchSet, PatchUpdate,
        },
        web::{interface::*, property_removals, util::deserialize_msgpack},
        SessionId,
    };
    use std::sync::Arc;

    fn metadata(mode: &str) -> InstanceMetadata {
        InstanceMetadata {
            effective_sync_authority: Some(Arc::new(
                serde_json::from_value(serde_json::json!([{"className":"Part","properties":mode}]))
                    .unwrap(),
            )),
            ..Default::default()
        }
    }

    #[test]
    fn queued_authority_survives_later_policy_changes_and_deletion() {
        let mut tree = RojoTree::new(InstanceSnapshot::new().name("Root").class_name("Folder"));
        let root = tree.get_root_id();
        let mut node = InstanceSnapshot::new().name("Pole").class_name("Part");
        node.metadata = metadata("studio");
        let added = apply_patch_set(
            &mut tree,
            PatchSet {
                added_instances: vec![PatchAdd {
                    parent_id: root,
                    instance: node,
                }],
                ..Default::default()
            },
        );
        let id = added.added[0];
        let filesystem = apply_patch_set(
            &mut tree,
            PatchSet {
                updated_instances: vec![PatchUpdate {
                    id,
                    changed_name: None,
                    changed_class_name: None,
                    changed_properties: Default::default(),
                    changed_metadata: Some(metadata("filesystem")),
                }],
                ..Default::default()
            },
        );
        let legacy = apply_patch_set(
            &mut tree,
            PatchSet {
                updated_instances: vec![PatchUpdate {
                    id,
                    changed_name: None,
                    changed_class_name: None,
                    changed_properties: Default::default(),
                    changed_metadata: Some(InstanceMetadata::default()),
                }],
                ..Default::default()
            },
        );
        apply_patch_set(
            &mut tree,
            PatchSet {
                removed_instances: vec![id],
                ..Default::default()
            },
        );
        let patches = vec![added, filesystem, legacy];
        let packet = SocketPacket {
            session_id: SessionId::new(),
            packet_type: SocketPacketType::Messages,
            body: SocketPacketBody::Messages(MessagesPacket {
                message_cursor: 3,
                messages: patches
                    .iter()
                    .map(SubscribeMessage::from_patch_update)
                    .collect(),
            }),
        };
        let bytes =
            property_removals::serialize_packet_with_authority(&packet, true, Some(&patches))
                .unwrap();
        let value: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
        for (index, mode) in ["studio", "filesystem", "legacy"].into_iter().enumerate() {
            assert_eq!(
                value["body"]["messages"][index]["classAuthority"][id.to_string()],
                mode
            );
        }
        let bytes =
            property_removals::serialize_packet_with_authority(&packet, true, None).unwrap();
        let value: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
        assert!(value["body"]["messages"][0].get("classAuthority").is_none());
        let _: SocketPacket<'_> = deserialize_msgpack(&bytes).unwrap();
    }

    #[test]
    fn negotiation_is_exact_and_configured_studio_requires_a_capable_client() {
        for query in [
            None,
            Some("classAuthorityV1=false"),
            Some("classAuthorityV1=True"),
            Some("notclassAuthorityV1=true"),
            Some("classAuthorityV1=trueish"),
        ] {
            assert!(!negotiated(query));
        }
        assert!(negotiated(Some("other=true&classAuthorityV1=true")));
        let mut snapshot = InstanceSnapshot::new().name("Root").class_name("Folder");
        snapshot.metadata = metadata("studio");
        let tree = RojoTree::new(snapshot);
        assert!(requires_client(&tree));
        assert_eq!(
            read(&tree, std::iter::once(tree.get_root_id())).unwrap()[&tree.get_root_id()],
            "legacy"
        );
    }
}
