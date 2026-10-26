use std::collections::HashMap;

use rbx_dom_weak::types::Ref;
use serde::Serialize;

use super::{
    interface::{Instance, InstanceUpdate, SocketPacket, SocketPacketBody, SocketPacketType},
    util::serialize_msgpack,
};
use crate::SessionId;

pub(super) const CAPABILITY: &str = "propertyRemovalsV1";

// Keep the extension in private wire adapters so existing Rust struct literals
// and clients that did not negotiate removals retain their current contract.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtendedUpdate<'a> {
    #[serde(flatten)]
    legacy: &'a InstanceUpdate,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    removed_properties: Vec<&'a str>,
}

#[derive(Serialize)]
struct ExtendedMessage<'a, 'data> {
    removed: &'a [Ref],
    added: &'a HashMap<Ref, Instance<'data>>,
    updated: Vec<ExtendedUpdate<'a>>,
    #[serde(rename = "classAuthority", skip_serializing_if = "Option::is_none")]
    class_authority: Option<HashMap<Ref, &'static str>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtendedBody<'a, 'data> {
    message_cursor: u32,
    messages: Vec<ExtendedMessage<'a, 'data>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtendedPacket<'a, 'data> {
    session_id: SessionId,
    packet_type: &'a SocketPacketType,
    body: ExtendedBody<'a, 'data>,
}

#[cfg(test)]
pub(super) fn serialize_packet(
    packet: &SocketPacket<'_>,
    property_removals: bool,
) -> anyhow::Result<Vec<u8>> {
    serialize_packet_with_authority(packet, property_removals, None)
}

pub(super) fn serialize_packet_with_authority(
    packet: &SocketPacket<'_>,
    property_removals: bool,
    authority: Option<&[crate::snapshot::AppliedPatchSet]>,
) -> anyhow::Result<Vec<u8>> {
    if !property_removals && authority.is_none() {
        return serialize_msgpack(packet);
    }
    let SocketPacketBody::Messages(body) = &packet.body;
    anyhow::ensure!(
        authority.is_none_or(|patches| patches.len() == body.messages.len()),
        "Class authority batch does not match its immutable patches"
    );
    let messages = body
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            Ok(ExtendedMessage {
                removed: &message.removed,
                added: &message.added,
                updated: message
                    .updated
                    .iter()
                    .map(|update| {
                        let mut removed_properties: Vec<_> = update
                            .changed_properties
                            .iter()
                            .filter_map(|(name, value)| {
                                (property_removals && value.is_none()).then_some(name.as_str())
                            })
                            .collect();
                        removed_properties.sort_unstable();
                        ExtendedUpdate {
                            legacy: update,
                            removed_properties,
                        }
                    })
                    .collect(),
                class_authority: authority
                    .map(|patches| {
                        patches[index]
                            .class_authority
                            .iter()
                            .map(|(id, policy)| {
                                let policy = policy
                                    .as_ref()
                                    .map_err(|error| anyhow::anyhow!(error.clone()))?;
                                Ok((*id, super::class_authority::label(*policy)))
                            })
                            .collect::<anyhow::Result<HashMap<_, _>>>()
                    })
                    .transpose()?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    serialize_msgpack(ExtendedPacket {
        session_id: packet.session_id,
        packet_type: &packet.packet_type,
        body: ExtendedBody {
            message_cursor: body.message_cursor,
            messages,
        },
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        web::{interface::*, util::deserialize_msgpack},
        SessionId,
    };
    use rbx_dom_weak::{
        types::{Ref, Variant},
        UstrMap,
    };
    use std::collections::HashMap;

    #[test]
    fn negotiated_removals_preserve_legacy_packets_and_distinguish_null_refs() {
        let mut packet = SocketPacket {
            session_id: SessionId::new(),
            packet_type: SocketPacketType::Messages,
            body: SocketPacketBody::Messages(MessagesPacket {
                message_cursor: 7,
                messages: vec![SubscribeMessage {
                    removed: Vec::new(),
                    added: HashMap::new(),
                    updated: vec![InstanceUpdate {
                        id: Ref::new(),
                        changed_name: None,
                        changed_class_name: None,
                        changed_metadata: None,
                        changed_properties: UstrMap::from_iter([
                            ("Transparency".into(), None),
                            ("A".into(), None),
                            ("Value".into(), Some(Variant::Ref(Ref::none()))),
                            ("Anchored".into(), Some(Variant::Bool(true))),
                        ]),
                    }],
                }],
            }),
        };
        let legacy = serialize_packet(&packet, false).unwrap();
        assert_eq!(legacy, serialize_msgpack(&packet).unwrap());
        let original: serde_json::Value = deserialize_msgpack(&legacy).unwrap();
        assert!(original["body"]["messages"][0]["updated"][0]
            .get("removedProperties")
            .is_none());
        let extended = serialize_packet(&packet, true).unwrap();
        let decoded: serde_json::Value = deserialize_msgpack(&extended).unwrap();
        let update = &decoded["body"]["messages"][0]["updated"][0];
        assert_eq!(
            update["removedProperties"],
            serde_json::json!(["A", "Transparency"])
        );
        assert_eq!(
            update["changedProperties"],
            original["body"]["messages"][0]["updated"][0]["changedProperties"]
        );
        let _: SocketPacket<'_> = deserialize_msgpack(&extended).unwrap();
        let SocketPacketBody::Messages(body) = &mut packet.body;
        body.messages[0].updated[0]
            .changed_properties
            .retain(|_, value| value.is_some());
        let no_removals = serialize_packet(&packet, true).unwrap();
        let decoded: serde_json::Value = deserialize_msgpack(&no_removals).unwrap();
        assert!(decoded["body"]["messages"][0]["updated"][0]
            .get("removedProperties")
            .is_none());
    }
}
