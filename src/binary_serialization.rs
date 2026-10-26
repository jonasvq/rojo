use std::{io::Write, sync::OnceLock};

use rbx_dom_weak::{types::Ref, types::Variant, WeakDom};
use rbx_reflection::ReflectionDatabase;

/// Preserve legacy physics migration when a sibling introduces volume columns.
/// Explicit per-instance values retain precedence.
pub(crate) fn to_writer<W: Write>(writer: W, dom: &WeakDom, refs: &[Ref]) -> anyhow::Result<()> {
    to_writer_with_compression(writer, dom, refs, rbx_binary::CompressionType::default())
}

pub(crate) fn to_writer_with_compression<W: Write>(
    writer: W,
    dom: &WeakDom,
    refs: &[Ref],
    compression: rbx_binary::CompressionType,
) -> anyhow::Result<()> {
    static DATABASE: OnceLock<Result<ReflectionDatabase<'static>, rbx_reflection_database::Error>> =
        OnceLock::new();
    let database = DATABASE
        .get_or_init(|| {
            rbx_reflection_database::get().map(|database| {
                let mut database = database.clone();
                // Binary properties are columns shared by every instance of a
                // class. A modern sibling can introduce UnscaledVolume into
                // the output for an older part that never stored it. Roblox
                // uses zero to compute volume from that part's legacy physics
                // data; the reflected near-zero positive default instead
                // becomes an actual volume, producing practically zero mass.
                for class in database.classes.values_mut() {
                    if let Some(value) = class.default_properties.get_mut("UnscaledVolume") {
                        *value = Variant::Float32(0.0);
                    }
                }
                database
            })
        })
        .as_ref()
        .map_err(|error| {
            anyhow::anyhow!("Failed to load binary serialization reflection database: {error}")
        })?;
    rbx_binary::Serializer::new()
        .reflection_database(database)
        .compression_type(compression)
        .serialize(writer, dom, refs)?;
    Ok(())
}
