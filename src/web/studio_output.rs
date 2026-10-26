use std::collections::HashMap;
use std::ops::Range;

use anyhow::{ensure, Result};
use rbx_dom_weak::types::Ref;
use serde::Deserialize;

use crate::{serve_session::ServeSession, SessionId};

pub(super) const CAPABILITY: &str = "studioOutputV1";
pub(super) const BODY_LIMIT: usize = 1024 * 1024;
pub(super) const LEVELS: [&str; 4] = ["Error", "Warning", "Info", "Output"];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Batch {
    pub session_id: SessionId,
    pub entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    level: Level,
    message: String,
}

#[derive(Clone, Copy, Deserialize)]
enum Level {
    Error,
    Warning,
    Info,
    Output,
}
impl Level {
    fn index(self) -> usize {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
            Self::Output => 3,
        }
    }
}

pub(super) fn format_batch(
    session: &ServeSession,
    enabled: [bool; 4],
    batch: Batch,
) -> Result<Vec<String>> {
    ensure!(batch.session_id == session.session_id(), "Wrong session ID");
    ensure!(
        !batch.entries.is_empty() && batch.entries.len() <= 64,
        "Studio output batch must contain 1..64 entries"
    );
    ensure!(
        batch
            .entries
            .iter()
            .all(|entry| entry.message.len() <= 8192),
        "Studio output message exceeds 8192 UTF-8 bytes"
    );
    let tree = session.tree();
    let mut scripts: HashMap<String, Vec<Ref>> = HashMap::new();
    for instance in tree.inner().descendants() {
        if matches!(
            instance.class.as_str(),
            "Script" | "LocalScript" | "ModuleScript"
        ) {
            scripts
                .entry(tree.inner().full_path_of(instance.referent(), "."))
                .or_default()
                .push(instance.referent());
        }
    }
    let mut cache = HashMap::new();
    let mut formatted = Vec::new();
    for entry in batch.entries {
        let level = entry.level.index();
        if !enabled[level] {
            continue;
        }
        for line in entry.message.split('\n') {
            let mapped = rewrite_line(line, |name| {
                let ids = scripts.get(name)?;
                if ids.len() != 1 {
                    return None;
                }
                let paths =
                    super::source_paths::paths_for(&tree, session.vfs(), ids[0], &mut cache)
                        .ok()?;
                let paths: Vec<_> = paths
                    .into_iter()
                    .filter(|path| {
                        std::path::Path::new(path)
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .is_some_and(|extension| {
                                extension.eq_ignore_ascii_case("lua")
                                    || extension.eq_ignore_ascii_case("luau")
                            })
                    })
                    .collect();
                (paths.len() == 1).then(|| paths[0].clone())
            });
            let safe = sanitize(&mapped);
            formatted.push(format!("[Studio {}] {safe}", LEVELS[level]));
        }
    }
    Ok(formatted)
}

fn rewrite_line(line: &str, mut source: impl FnMut(&str) -> Option<String>) -> String {
    let trimmed = line.trim_start();
    let start = line.len() - trimmed.len();
    let mut candidates: Vec<(Range<usize>, String)> = Vec::new();
    if let Some(rest) = trimmed.strip_prefix("Script '") {
        if let Some((name, tail)) = rest.rsplit_once("', Line ") {
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(number) = tail[..digits].parse::<u32>() {
                if number > 0 {
                    if let Some(path) = source(name) {
                        let end = line.len() - tail.len() + digits;
                        candidates.push((start..end, format!("{path}:{number}")));
                    }
                }
            }
        }
    } else {
        for (colon, _) in trimmed.match_indices(':') {
            let tail = &trimmed[colon + 1..];
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            if tail.as_bytes().get(digits) != Some(&b':') {
                continue;
            }
            if let Ok(number) = tail[..digits].parse::<u32>() {
                if number > 0 {
                    if let Some(path) = source(&trimmed[..colon]) {
                        candidates.push((
                            start..start + colon + 1 + digits,
                            format!("{path}:{number}"),
                        ));
                    }
                }
            }
        }
    }
    if candidates.len() != 1 {
        return line.to_owned();
    }
    let (range, replacement) = candidates.pop().unwrap();
    format!(
        "{}{}{}",
        &line[..range.start],
        replacement,
        &line[range.end..]
    )
}

fn sanitize(text: &str) -> String {
    let mut result = String::new();
    for character in text.chars() {
        if character.is_control() && !matches!(character, '\n' | '\t') {
            result.extend(character.escape_default());
        } else {
            result.push(character);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_report_forms_and_preserves_lines_suffixes_and_unknown_text() {
        let resolve = |name: &str| {
            (name == "ReplicatedFirst.runtime")
                .then(|| "F:/Project/src/runtime/init.client.luau".into())
        };
        assert_eq!(
            rewrite_line(
                "ReplicatedFirst.runtime:27: attempt to call a nil value",
                resolve
            ),
            "F:/Project/src/runtime/init.client.luau:27: attempt to call a nil value"
        );
        assert_eq!(
            rewrite_line(
                "  Script 'ReplicatedFirst.runtime', Line 27 - function run",
                resolve
            ),
            "  F:/Project/src/runtime/init.client.luau:27 - function run"
        );
        for text in [
            "Stack Begin",
            "Stack End",
            "Unknown:27: error",
            "ReplicatedFirst.runtime:0: error",
            "ReplicatedFirst.runtime:999999999999: error",
        ] {
            assert_eq!(rewrite_line(text, resolve), text);
        }
    }

    #[test]
    fn terminal_controls_are_escaped_while_tabs_and_multiline_text_survive() {
        let text = sanitize("\x1b[31mred\r\x08\tline\nnext");
        assert!(!text.contains('\x1b') && !text.contains('\r') && !text.contains('\x08'));
        assert!(text.contains("\tline\nnext"));
    }

    fn session() -> ServeSession {
        use memofs::{InMemoryFs, Vfs, VfsSnapshot};
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/output-project",VfsSnapshot::dir([
            ("default.project.json",VfsSnapshot::file(r#"{"name":"Output","tree":{"$className":"DataModel","ReplicatedFirst":{"$className":"ReplicatedFirst","$path":"src","embedded":{"$className":"ModuleScript","$properties":{"Source":"return 1"}}}}}"#)),
            ("src",VfsSnapshot::dir([("runtime",VfsSnapshot::dir([("init.client.luau",VfsSnapshot::file("print('hello')"))]))])),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        vfs.set_watch_enabled(false);
        ServeSession::new(vfs, "/output-project/default.project.json").unwrap()
    }

    fn batch(session: &ServeSession, message: &str) -> Batch {
        Batch {
            session_id: session.session_id(),
            entries: vec![Entry {
                level: Level::Error,
                message: message.into(),
            }],
        }
    }

    #[test]
    fn current_tree_paths_map_init_sources_and_keep_embedded_missing_and_duplicate_locations() {
        use crate::snapshot::InstanceSnapshot;
        let session = session();
        let text="ReplicatedFirst.runtime:27: failed\nScript 'ReplicatedFirst.runtime', Line 27\nReplicatedFirst.embedded:1: untouched";
        let output = format_batch(&session, [true; 4], batch(&session, text)).unwrap();
        assert!(
            output[0].contains("/output-project/src/runtime/init.client.luau:27: failed"),
            "{output:?}"
        );
        assert!(output[1].contains("/output-project/src/runtime/init.client.luau:27"));
        assert!(output[2].ends_with("ReplicatedFirst.embedded:1: untouched"));
        let id = {
            let tree = session.tree();
            tree.inner()
                .descendants()
                .find(|instance| instance.name == "runtime")
                .unwrap()
                .referent()
        };
        {
            let mut tree = session.tree();
            let instance = tree.get_instance(id).unwrap();
            let (parent, class, metadata) = (
                instance.parent(),
                instance.class_name(),
                instance.metadata().clone(),
            );
            tree.insert_instance(
                parent,
                InstanceSnapshot::new()
                    .name("runtime")
                    .class_name(class)
                    .metadata(metadata),
            );
        }
        let output = format_batch(
            &session,
            [true; 4],
            batch(&session, "ReplicatedFirst.runtime:27: duplicate"),
        )
        .unwrap();
        assert!(output[0].ends_with("ReplicatedFirst.runtime:27: duplicate"));
        let session = self::session();
        session
            .vfs()
            .remove_file("/output-project/src/runtime/init.client.luau")
            .unwrap();
        let output = format_batch(
            &session,
            [true; 4],
            batch(&session, "ReplicatedFirst.runtime:27: missing"),
        )
        .unwrap();
        assert!(output[0].ends_with("ReplicatedFirst.runtime:27: missing"));
    }

    #[test]
    fn invalid_batch_is_rejected_as_a_whole_and_disabled_levels_are_filtered() {
        let session = session();
        let mut bad = batch(&session, "first valid line");
        bad.entries.push(Entry {
            level: Level::Error,
            message: "x".repeat(8193),
        });
        assert!(format_batch(&session, [true; 4], bad).is_err());
        assert!(
            format_batch(&session, [false; 4], batch(&session, "filtered"))
                .unwrap()
                .is_empty()
        );
        assert!(format_batch(
            &session,
            [true; 4],
            Batch {
                session_id: SessionId::new(),
                entries: vec![]
            }
        )
        .is_err());
    }
}
