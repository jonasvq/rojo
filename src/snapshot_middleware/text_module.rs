use std::{fmt::Write as _, path::Path};

use anyhow::{ensure, Context};
use memofs::Vfs;
use rbx_dom_weak::{types::Variant, ustr};

use crate::{
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot},
    syncback::{FsSnapshot, SyncbackReturn, SyncbackSnapshot},
};

use super::{meta_file::AdjacentMetadata, Middleware, PathExt};

pub(crate) fn encode_text(contents: &str) -> String {
    let mut source = String::from("return \"");
    for character in contents.chars() {
        match character {
            '"' => source.push_str("\\\""),
            '\\' => source.push_str("\\\\"),
            control if control.is_ascii_control() => {
                // Fixed width prevents a following digit becoming part of the escape.
                write!(source, "\\{:03}", control as u8).unwrap();
            }
            other => source.push(other),
        }
    }
    source.push_str("\"\n");
    source
}

pub(crate) fn decode_text(source: &str) -> anyhow::Result<String> {
    let literal = source
        .strip_prefix("return \"")
        .and_then(|value| value.strip_suffix("\"\n"))
        .context("textModule Source must be the canonical return string literal")?;
    let mut input = literal.bytes();
    let mut bytes = Vec::with_capacity(literal.len());
    while let Some(byte) = input.next() {
        match byte {
            b'"' => anyhow::bail!("unescaped quote in textModule literal"),
            b'\\' => match input.next().context("incomplete textModule escape")? {
                escaped @ (b'"' | b'\\') => bytes.push(escaped),
                first @ b'0'..=b'9' => {
                    let second = input.next().context("incomplete decimal escape")?;
                    let third = input.next().context("incomplete decimal escape")?;
                    ensure!(
                        second.is_ascii_digit() && third.is_ascii_digit(),
                        "invalid decimal escape"
                    );
                    let value = u16::from(first - b'0') * 100
                        + u16::from(second - b'0') * 10
                        + u16::from(third - b'0');
                    bytes.push(u8::try_from(value).context("decimal escape exceeds one byte")?);
                }
                _ => anyhow::bail!("noncanonical textModule escape"),
            },
            other => bytes.push(other),
        }
    }
    let text = String::from_utf8(bytes).context("textModule literal is not UTF-8")?;
    ensure!(
        encode_text(&text) == source,
        "textModule Source is not the canonical return string literal"
    );
    Ok(text)
}

pub(super) fn snapshot_text_module(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let contents = vfs
        .read_to_string(path)
        .with_context(|| format!("Could not read UTF-8 textModule {}", path.display()))?;
    let mut snapshot = InstanceSnapshot::new()
        .name(name)
        .class_name("ModuleScript")
        .property(ustr("Source"), encode_text(&contents))
        .metadata(
            InstanceMetadata::new()
                .instigating_source(path)
                .relevant_paths(vec![vfs.canonicalize(path)?])
                .context(context),
        );
    AdjacentMetadata::read_and_apply_all(vfs, path, name, &mut snapshot)?;
    Ok(Some(snapshot))
}

pub(super) fn syncback_text_module<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let instance = snapshot.new_inst();
    ensure!(instance.class == "ModuleScript" && instance.children().is_empty(),
        "Cannot change textModule class or add children at {}; explicit source-layout migration required", snapshot.path.display());
    let old = snapshot
        .old_inst()
        .context("textModule syncback requires a known file binding")?;
    let rule = old
        .metadata()
        .context
        .get_user_sync_rule(&snapshot.path)
        .filter(|rule| rule.middleware == Some(Middleware::TextModule))
        .context("textModule destination no longer matches its source rule")?;
    let meta_name = rule.file_name_for_path(&snapshot.path)?;
    let Variant::String(source) = instance
        .properties
        .get(&ustr("Source"))
        .context("textModule requires a string Source")?
    else {
        anyhow::bail!(
            "textModule requires a string Source at {}",
            snapshot.path.display()
        );
    };
    let contents = decode_text(source).with_context(|| {
        format!(
            "Cannot syncback textModule {} at {}",
            instance.name,
            snapshot.path.display()
        )
    })?;
    let mut fs_snapshot = FsSnapshot::new();
    fs_snapshot.add_file(&snapshot.path, contents.into_bytes());
    if let Some(mut meta) =
        AdjacentMetadata::from_syncback_snapshot(snapshot, snapshot.path.clone())?
    {
        meta.id = old
            .metadata()
            .specified_id
            .as_ref()
            .map(ToString::to_string);
        meta.properties.shift_remove(&ustr("Source"));
        let path = super::lua::metadata_destination(
            snapshot.vfs(),
            &snapshot
                .path
                .parent_err()?
                .join(format!("{meta_name}.meta.json")),
        )?;
        if !meta.is_empty() || snapshot.vfs().exists(&path)? {
            fs_snapshot.add_file(path, serde_json::to_vec_pretty(&meta)?);
        }
    }
    Ok(SyncbackReturn {
        fs_snapshot,
        children: Vec::new(),
        removed_children: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_roundtrips_exact_utf8_and_adversarial_bytes() {
        for text in [
            "",
            "\u{feff}日本語 ä 😀",
            "\r\n\n\r",
            "\0",
            "\x00123456789",
            "\t\u{7f}",
            "\"\\",
            "]] ]=] ]==] -- ",
            "\nfirst\nlast",
            "no final newline",
        ] {
            assert_eq!(decode_text(&encode_text(text)).unwrap(), text);
        }
        let all_ascii: String = (0..=127).map(char::from).collect();
        assert_eq!(decode_text(&encode_text(&all_ascii)).unwrap(), all_ascii);
    }

    #[test]
    fn decoder_rejects_code_noncanonical_literals_and_invalid_utf8() {
        for source in [
            "return 'text'\n",
            "return [[text]]\n",
            "return 42",
            "return \"x\"; print(1)\n",
            "return \"x\"\n-- comment",
            "return \"\\065\"\n",
            "return \"\\256\"\n",
            "return \"\\999\"\n",
            "return \"\\255\"\n",
            "return \"\\1\"\n",
            "return \"\\n\"\n",
            "return \"\\u{41}\"\n",
            "return \"literal\nnewline\"\n",
            "return \"unterminated",
            "return \"x\"\r\n",
        ] {
            assert!(decode_text(source).is_err(), "accepted {source:?}");
        }
    }
}
