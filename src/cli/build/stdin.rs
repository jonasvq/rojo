use std::{io::Read, path::Path};

use anyhow::Context;
use memofs::{IoResultExt, Vfs};

use crate::{Project, SessionId};

pub(super) fn is_stdin(path: &Path) -> bool {
    path == Path::new("-")
}

pub(super) fn read_project(vfs: &Vfs) -> anyhow::Result<Project> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .read_to_end(&mut bytes)
        .context("Could not read project from stdin")?;
    let base = vfs.canonicalize(std::env::current_dir()?)?;
    let origin = loop {
        let candidate = base.join(format!(".rojo-stdin-{}.project.json", SessionId::new()));
        if vfs.metadata(&candidate).with_not_found()?.is_none() {
            break candidate;
        }
    };
    let name = base
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("stdin");
    Project::from_stdin_bytes(&bytes, origin, name).context("Invalid project JSON from stdin")
}
