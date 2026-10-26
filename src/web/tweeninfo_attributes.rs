use std::borrow::Cow;

use rbx_dom_weak::types::Variant;

use super::interface::{Instance, SubscribeMessage};

pub(super) const CAPABILITY: &str = "attributeTweenInfoV1";

pub(super) fn negotiated(query: Option<&str>) -> bool {
    query.is_some_and(|query| {
        query
            .split('&')
            .any(|part| part == "attributeTweenInfoV1=true")
    })
}

fn legacy_value(value: &Variant) -> anyhow::Result<Option<Variant>> {
    if let Variant::Attributes(attributes) = value {
        if attributes
            .iter()
            .any(|(_, value)| matches!(value, Variant::TweenInfo(_)))
        {
            let mut bytes = Vec::new();
            attributes.to_writer(&mut bytes)?;
            // Preserve the entire blob for clients whose nested value decoder
            // would otherwise silently discard the new attribute type.
            return Ok(Some(Variant::BinaryString(bytes.into())));
        }
    }
    Ok(None)
}

pub(super) fn adapt_instance(instance: &mut Instance<'_>, supported: bool) -> anyhow::Result<()> {
    if !supported {
        for value in instance.properties.values_mut() {
            if let Some(legacy) = legacy_value(value)? {
                *value = Cow::Owned(legacy);
            }
        }
    }
    Ok(())
}

pub(super) fn adapt_message(
    message: &mut SubscribeMessage<'_>,
    supported: bool,
) -> anyhow::Result<()> {
    if !supported {
        for instance in message.added.values_mut() {
            adapt_instance(instance, false)?;
        }
        for update in &mut message.updated {
            for value in update.changed_properties.values_mut().flatten() {
                if let Some(legacy) = legacy_value(value)? {
                    *value = legacy;
                }
            }
        }
    }
    Ok(())
}
