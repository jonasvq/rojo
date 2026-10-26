use std::collections::HashMap;

use anyhow::Context;
use hyper::{Body, Response};
use memofs::{IoResultExt, Vfs};
use rbx_dom_weak::types::Ref;
use serde::Serialize;

use crate::{path_serializer, snapshot::RojoTree, SessionId};

use super::{
    interface::{Instance, ReadResponse},
    util::msgpack_ok,
};

pub(super) fn negotiated(query: Option<&str>) -> bool {
    query.is_some_and(|query| query.split('&').any(|part| part == "showPaths=true"))
}

#[derive(Serialize)]
struct InstanceWithPaths<'a> {
    #[serde(flatten)]
    instance: Instance<'a>,
    #[serde(rename = "FilePaths")]
    file_paths: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadWithPaths<'a> {
    session_id: SessionId,
    message_cursor: u32,
    instances: HashMap<Ref, InstanceWithPaths<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    write_sources: Option<HashMap<Ref, crate::write_receipts::BindingView>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    class_authority: Option<HashMap<Ref, &'static str>>,
}

pub(super) fn encode_read_with_authority(
    tree: &RojoTree,
    vfs: &Vfs,
    response: ReadResponse<'_>,
    write_sources: Option<HashMap<Ref, crate::write_receipts::BindingView>>,
    class_authority: Option<HashMap<Ref, &'static str>>,
) -> anyhow::Result<Response<Body>> {
    let mut cache = HashMap::new();
    let mut instances = HashMap::with_capacity(response.instances.len());
    for (id, instance) in response.instances {
        let file_paths = paths_for(tree, vfs, id, &mut cache)?;
        instances.insert(
            id,
            InstanceWithPaths {
                instance,
                file_paths,
            },
        );
    }
    Ok(msgpack_ok(ReadWithPaths {
        session_id: response.session_id,
        message_cursor: response.message_cursor,
        instances,
        write_sources,
        class_authority,
    }))
}

pub(super) fn paths_for(
    tree: &RojoTree,
    vfs: &Vfs,
    mut id: Ref,
    cache: &mut HashMap<Ref, Vec<String>>,
) -> anyhow::Result<Vec<String>> {
    let mut trail = Vec::new();
    let paths = loop {
        if let Some(paths) = cache.get(&id) {
            break paths.clone();
        }
        let Some(instance) = tree.get_instance(id) else {
            break Vec::new();
        };
        trail.push(id);
        let metadata = instance.metadata();
        if metadata.instigating_source.is_none() && metadata.relevant_paths.is_empty() {
            // Embedded model children share their nearest source, without
            // repeatedly walking a deep ancestor chain for each descendant.
            id = instance.parent();
            continue;
        }
        let mut candidates = metadata.relevant_paths.clone();
        if let Some(source) = &metadata.instigating_source {
            candidates.push(source.path().to_owned());
        }
        let mut paths = Vec::new();
        for path in candidates {
            let metadata = vfs
                .metadata(&path)
                .with_not_found()
                .with_context(|| format!("Could not inspect API source path {}", path.display()))?;
            if metadata.is_some_and(|metadata| metadata.is_file()) {
                let canonical = vfs.canonicalize(&path).with_not_found().with_context(|| {
                    format!("Could not resolve API source path {}", path.display())
                })?;
                if let Some(canonical) = canonical {
                    paths.push(path_serializer::display_absolute(canonical));
                }
            }
        }
        paths.sort();
        paths.dedup();
        break paths;
    };
    for id in trail {
        cache.insert(id, paths.clone());
    }
    Ok(paths)
}
