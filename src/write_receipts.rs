use std::{
    collections::HashMap,
    num::NonZeroUsize,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{ensure, Context};
use crossbeam_channel::Sender;
use futures::channel::oneshot;
use hyper::{Body, Method, Request, Response, StatusCode};
use memofs::Vfs;
use rbx_dom_weak::types::{Ref, Variant};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    change_processor::script_source_path,
    serve_session::ServeSession,
    snapshot::{InstigatingSource, RojoTree},
    snapshot_middleware::{decode_text_module, encode_text_module, Middleware},
    web::{
        api::request_bytes,
        interface::ErrorResponse,
        util::{msgpack, msgpack_ok},
    },
    SessionId,
};

pub(crate) const CAPABILITY: &str = "writeReceiptsV1";
pub(crate) const PENDING_LIMIT: usize = 64;
const TERMINAL_LIMIT: usize = 1024;
const RETENTION: Duration = Duration::from_secs(600);
const PENDING_BYTES: usize = 64 * 1024 * 1024;
pub(crate) type Shared = Arc<Mutex<Broker>>;

pub(crate) struct Processor {
    pub receiver: crossbeam_channel::Receiver<Command>,
    pub session_id: SessionId,
    pub broker: Shared,
}

pub(crate) enum Command {
    Read(Ref, oneshot::Sender<anyhow::Result<SourceView>>),
    Write(Uuid, WriteRequest),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WriteRequest {
    pub session_id: SessionId,
    pub origin_id: Uuid,
    pub request_id: Uuid,
    pub instance_id: Ref,
    pub class_name: String,
    pub source_hash: String,
    pub binding_hash: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceView {
    pub session_id: SessionId,
    pub instance_id: Ref,
    pub class_name: String,
    pub source_hash: String,
    pub binding_hash: String,
    pub source: String,
    pub instance_path: String,
    pub source_path: String,
}

pub(crate) struct WritableSource {
    pub view: SourceView,
    path: PathBuf,
    text_module: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindingView {
    pub class_name: String,
    pub source_hash: String,
    pub binding_hash: String,
}

pub(crate) fn read_bindings(
    session: SessionId,
    tree: &RojoTree,
    vfs: &Vfs,
    ids: impl Iterator<Item = Ref>,
) -> HashMap<Ref, BindingView> {
    let owners = source_owners(tree, vfs);
    ids.filter_map(|id| {
        read_source_with_owners(session, tree, vfs, id, &owners)
            .ok()
            .map(|source| {
                (
                    id,
                    BindingView {
                        class_name: source.view.class_name,
                        source_hash: source.view.source_hash,
                        binding_hash: source.view.binding_hash,
                    },
                )
            })
    })
    .collect()
}

fn source_owners(tree: &RojoTree, vfs: &Vfs) -> HashMap<PathBuf, usize> {
    let mut owners = HashMap::new();
    for instance in tree.descendants(tree.get_root_id()) {
        if let Ok(path) = script_source_path(vfs, instance.metadata())
            .and_then(|path| Ok(vfs.canonicalize(path)?))
        {
            *owners.entry(path).or_insert(0) += 1;
        }
    }
    owners
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum State {
    Pending,
    Applied,
    Unchanged,
    Conflict,
    Rejected,
    IoError,
    Uncertain,
    ProcessorStopped,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Outcome {
    pub state: State,
    pub reason: String,
    pub may_have_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

impl Outcome {
    pub fn new(state: State, reason: &str) -> Self {
        Self {
            state,
            reason: bounded(reason),
            may_have_changed: false,
            source_hash: None,
            binding_hash: None,
            cursor: None,
            instance_path: None,
            source_path: None,
        }
    }
    pub fn observed(mut self, source: &SourceView, cursor: u32) -> Self {
        self.source_hash = Some(source.source_hash.clone());
        self.binding_hash = Some(source.binding_hash.clone());
        self.cursor = Some(cursor);
        self.instance_path = Some(source.instance_path.clone());
        self.source_path = Some(source.source_path.clone());
        self
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReceiptView {
    session_id: SessionId,
    pub(crate) receipt_id: Uuid,
    origin_id: Uuid,
    request_id: Uuid,
    #[serde(flatten)]
    pub(crate) outcome: Outcome,
}

struct Entry {
    view: ReceiptView,
    digest: blake3::Hash,
    source_bytes: usize,
    completed: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct Broker {
    entries: HashMap<Uuid, Entry>,
    requests: HashMap<(Uuid, Uuid), Uuid>,
    stopped: bool,
}

impl Broker {
    fn prune(&mut self, now: Instant) {
        let mut completed: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| entry.completed.map(|time| (*id, time)))
            .collect();
        completed.sort_by_key(|(_, time)| *time);
        let excess = completed.len().saturating_sub(TERMINAL_LIMIT);
        for (index, (id, time)) in completed.into_iter().enumerate() {
            if index < excess || now.duration_since(time) >= RETENTION {
                self.remove(id);
            }
        }
    }

    fn remove(&mut self, id: Uuid) {
        if let Some(entry) = self.entries.remove(&id) {
            self.requests
                .remove(&(entry.view.origin_id, entry.view.request_id));
        }
    }

    pub(crate) fn admit(
        &mut self,
        request: WriteRequest,
        sender: &Sender<Command>,
        now: Instant,
    ) -> Result<ReceiptView, (StatusCode, &'static str)> {
        self.prune(now);
        let digest = blake3::hash(
            &serde_json::to_vec(&request)
                .expect("Receipt request contains only serializable scalars"),
        );
        if let Some(id) = self.requests.get(&(request.origin_id, request.request_id)) {
            let entry = &self.entries[id];
            return if entry.digest == digest {
                Ok(entry.view.clone())
            } else {
                Err((
                    StatusCode::CONFLICT,
                    "Request ID was reused with different content",
                ))
            };
        }
        if self.stopped {
            return Err((StatusCode::SERVICE_UNAVAILABLE, "Change processor stopped"));
        }
        let mut pending = self
            .entries
            .values()
            .filter(|entry| entry.completed.is_none());
        let (count, bytes) = pending.by_ref().fold((0, 0usize), |(count, bytes), entry| {
            (count + 1, bytes.saturating_add(entry.source_bytes))
        });
        if count >= PENDING_LIMIT || bytes.saturating_add(request.source.len()) > PENDING_BYTES {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "Write receipt queue or pending source byte budget is full",
            ));
        }
        let id = Uuid::new_v4();
        let view = ReceiptView {
            session_id: request.session_id,
            receipt_id: id,
            origin_id: request.origin_id,
            request_id: request.request_id,
            outcome: Outcome::new(State::Pending, "Queued; persistence is not yet confirmed"),
        };
        self.entries.insert(
            id,
            Entry {
                view: view.clone(),
                digest,
                source_bytes: request.source.len(),
                completed: None,
            },
        );
        self.requests
            .insert((request.origin_id, request.request_id), id);
        if let Err(error) = sender.try_send(Command::Write(id, request)) {
            self.remove(id);
            return Err(if error.is_full() {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    "Change processor queue is full",
                )
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, "Change processor stopped")
            });
        }
        Ok(view)
    }

    pub fn complete(&mut self, id: Uuid, outcome: Outcome) {
        if let Some(entry) = self.entries.get_mut(&id) {
            if entry.completed.is_none() {
                entry.view.outcome = outcome;
                entry.completed = Some(Instant::now());
                entry.source_bytes = 0;
            }
        }
        self.prune(Instant::now());
    }

    #[cfg(test)]
    pub(crate) fn test_outcome(&self, id: Uuid) -> Outcome {
        self.entries[&id].view.outcome.clone()
    }
}

#[cfg(test)]
thread_local! { pub(crate) static AFTER_WRITE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }

pub(crate) struct ProcessorGuard(pub Shared);
impl Drop for ProcessorGuard {
    fn drop(&mut self) {
        let mut broker = self.0.lock().unwrap();
        broker.stopped = true;
        for entry in broker
            .entries
            .values_mut()
            .filter(|entry| entry.completed.is_none())
        {
            entry.view.outcome = Outcome::new(
                State::ProcessorStopped,
                "Change processor stopped; persistence was not confirmed",
            );
            entry.view.outcome.may_have_changed = true;
            entry.completed = Some(Instant::now());
            entry.source_bytes = 0;
        }
        broker.prune(Instant::now());
    }
}

pub(crate) fn bounded(text: &str) -> String {
    let mut end = text.len().min(512);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub(crate) fn read_source(
    session: SessionId,
    tree: &RojoTree,
    vfs: &Vfs,
    id: Ref,
) -> anyhow::Result<WritableSource> {
    read_source_with_owners(session, tree, vfs, id, &source_owners(tree, vfs))
}

fn read_source_with_owners(
    session: SessionId,
    tree: &RojoTree,
    vfs: &Vfs,
    id: Ref,
    owners: &HashMap<PathBuf, usize>,
) -> anyhow::Result<WritableSource> {
    let instance = tree
        .get_instance(id)
        .context("Managed instance no longer exists")?;
    ensure!(
        matches!(
            instance.class_name().as_str(),
            "Script" | "LocalScript" | "ModuleScript"
        ),
        "Managed instance is not a native script or textModule"
    );
    let metadata = instance.metadata();
    ensure_legacy_write_authority(metadata, instance.class_name().as_str())?;
    if let Some(InstigatingSource::ProjectNode { node, .. }) = &metadata.instigating_source {
        ensure!(
            !node.properties.contains_key(&"Source".into()),
            "Project overrides Source; direct file writes are not reversible"
        );
    }
    let path = vfs.canonicalize(script_source_path(vfs, metadata)?)?;
    ensure!(
        owners.get(&path) == Some(&1),
        "Source has multiple managed owners; receipt writes require one exact binding"
    );
    for relevant in &metadata.relevant_paths {
        let Some(name) = relevant.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.ends_with(".meta.json") || name.ends_with(".meta.jsonc") {
            if let Some(bytes) = vfs.read(relevant).with_not_found()? {
                let value: serde_json::Value = crate::json::from_slice(&bytes)?;
                ensure!(
                    value
                        .get("properties")
                        .and_then(|value| value.as_object())
                        .is_none_or(|properties| !properties.contains_key("Source")),
                    "Metadata overrides Source; direct file writes are not reversible"
                );
            }
        }
    }
    let text_module = metadata.middleware == Some(Middleware::TextModule);
    let source = if text_module {
        encode_text_module(&vfs.read_to_string(&path)?)
    } else {
        vfs.read_to_string_lf_normalized(&path)?.as_str().to_owned()
    };
    ensure!(
        instance.properties().get(&"Source".into()) == Some(&Variant::String(source.clone())),
        "Source changed during snapshot or is overridden; refresh before writing"
    );
    let mut names = Vec::new();
    let mut current = Some(instance);
    while let Some(instance) = current {
        names.push(instance.name().to_owned());
        current = tree.get_instance(instance.parent());
    }
    names.reverse();
    let binding = serde_json::to_vec(&(
        path.to_string_lossy(),
        &names,
        instance.class_name().as_str(),
        metadata.middleware,
        &metadata.context,
        &metadata.rule_defaults,
    ))?;
    Ok(WritableSource {
        view: SourceView {
            session_id: session,
            instance_id: id,
            class_name: instance.class_name().to_string(),
            source_hash: blake3::hash(source.as_bytes()).to_hex().to_string(),
            binding_hash: blake3::hash(&binding).to_hex().to_string(),
            source,
            instance_path: bounded(&names.join(".")),
            source_path: bounded(&path.display().to_string()),
        },
        path,
        text_module,
    })
}

pub(crate) fn ensure_legacy_write_authority(
    metadata: &crate::InstanceMetadata,
    class_name: &str,
) -> anyhow::Result<()> {
    let policy = metadata
        .effective_sync_authority
        .as_ref()
        .map(|authority| authority.resolve(class_name))
        .transpose()?
        .flatten();
    ensure!(
        policy.is_none(),
        "Explicit class authority forbids legacy writes and Source-only receipts"
    );
    Ok(())
}

impl WritableSource {
    pub fn requested_source(&self, source: &str) -> anyhow::Result<String> {
        if self.text_module {
            decode_text_module(source)?;
            Ok(source.to_owned())
        } else {
            Ok(source.replace("\r\n", "\n"))
        }
    }
    pub fn write(&self, vfs: &Vfs, source: &str) -> anyhow::Result<()> {
        let contents = if self.text_module {
            std::borrow::Cow::Owned(decode_text_module(source)?)
        } else {
            std::borrow::Cow::Borrowed(source)
        };
        vfs.write(&self.path, contents.as_bytes())
            .context("Could not write source file")?;
        #[cfg(test)]
        AFTER_WRITE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().take() {
                callback();
            }
        });
        Ok(())
    }
}

use memofs::IoResultExt as _;

pub(crate) async fn handle(
    session: &ServeSession,
    request: Request<Body>,
    limit: NonZeroUsize,
) -> Response<Body> {
    let session_id = session.session_id();
    let path = request.uri().path().to_owned();
    let error = |code, message: &str| msgpack(ErrorResponse::bad_request(bounded(message)), code);
    if request.method() == Method::GET {
        if let Some(id) = path.strip_prefix("/api/write-receipts/source/") {
            let Ok(id) = id.parse() else {
                return error(StatusCode::BAD_REQUEST, "Invalid managed instance ID");
            };
            let (sender, receiver) = oneshot::channel();
            if let Err(failure) = session.receipt_sender().try_send(Command::Read(id, sender)) {
                return error(
                    if failure.is_full() {
                        StatusCode::TOO_MANY_REQUESTS
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    },
                    "Source read queue is unavailable",
                );
            }
            return match tokio::time::timeout(Duration::from_secs(5), receiver).await {
                Ok(Ok(Ok(source))) => msgpack_ok(source),
                Ok(Ok(Err(_))) => error(StatusCode::CONFLICT, "Source binding is missing, changed, ambiguous or not reversible; inspect the serve terminal"),
                _ => error(StatusCode::SERVICE_UNAVAILABLE, "Source read did not complete; no write was requested"),
            };
        }
        if let Some(id) = path.strip_prefix("/api/write-receipts/") {
            let Ok(id) = Uuid::parse_str(id) else {
                return error(StatusCode::BAD_REQUEST, "Invalid receipt ID");
            };
            let mut query = HashMap::new();
            for pair in request.uri().query().unwrap_or("").split('&') {
                let Some((key, value)) = pair.split_once('=') else {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "Receipt query requires sessionId and originId",
                    );
                };
                if query.insert(key, value).is_some() {
                    return error(StatusCode::BAD_REQUEST, "Duplicate receipt query field");
                }
            }
            if query.len() != 2
                || query.get("sessionId").copied() != Some(session_id.to_string().as_str())
            {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Wrong session ID or receipt query fields",
                );
            }
            let Some(origin) = query
                .get("originId")
                .and_then(|id| Uuid::parse_str(id).ok())
            else {
                return error(StatusCode::BAD_REQUEST, "Invalid origin ID");
            };
            let shared = session.receipts();
            let view = {
                let mut broker = shared.lock().unwrap();
                broker.prune(Instant::now());
                broker
                    .entries
                    .get(&id)
                    .filter(|entry| entry.view.origin_id == origin)
                    .map(|entry| entry.view.clone())
            };
            return match view { Some(view) => msgpack_ok(view), None => error(StatusCode::NOT_FOUND, "Receipt missing or expired; persistence is uncertain, refresh and review before retrying") };
        }
    } else if request.method() == Method::POST && path == "/api/write-receipts" {
        let body = match request_bytes(request, limit).await {
            Ok(body) => body,
            Err((failure, status)) => return msgpack(failure, status),
        };
        let mut decoder =
            rmp_serde::Deserializer::new(std::io::Cursor::new(&body)).with_human_readable();
        let parsed = WriteRequest::deserialize(&mut decoder);
        let request = match parsed {
            Ok(request) if decoder.get_ref().position() == body.len() as u64 => request,
            _ => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Invalid receipt request fields or trailing MessagePack data",
                )
            }
        };
        if request.session_id != session_id {
            return error(StatusCode::BAD_REQUEST, "Wrong session ID");
        }
        if request.origin_id.is_nil()
            || request.request_id.is_nil()
            || request.class_name.len() > 128
            || [&request.source_hash, &request.binding_hash]
                .iter()
                .any(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return error(
                StatusCode::BAD_REQUEST,
                "Invalid receipt identity, class or hashes",
            );
        }
        let shared = session.receipts();
        return match shared.lock().unwrap().admit(
            request,
            &session.receipt_sender(),
            Instant::now(),
        ) {
            Ok(receipt) => msgpack(receipt, StatusCode::ACCEPTED),
            Err((status, message)) => error(status, message),
        };
    }
    error(StatusCode::NOT_FOUND, "Unknown write receipt route")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> WriteRequest {
        WriteRequest {
            session_id: SessionId::new(),
            origin_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            instance_id: Ref::new(),
            class_name: "ModuleScript".into(),
            source_hash: "a".repeat(64),
            binding_hash: "b".repeat(64),
            source: "return 1".into(),
        }
    }

    #[test]
    fn receipt_queue_deduplicates_pending_requests_and_bounds_work() {
        let mut broker = Broker::default();
        let (sender, receiver) = crossbeam_channel::bounded(PENDING_LIMIT);
        let request = request();
        let first = broker
            .admit(request.clone(), &sender, Instant::now())
            .unwrap();
        let second = broker
            .admit(request.clone(), &sender, Instant::now())
            .unwrap();
        assert_eq!(first.receipt_id, second.receipt_id);
        assert_eq!(receiver.len(), 1);
        let mut changed = request.clone();
        changed.source.push('2');
        assert_eq!(
            broker
                .admit(changed, &sender, Instant::now())
                .unwrap_err()
                .0,
            StatusCode::CONFLICT
        );
        for _ in 1..PENDING_LIMIT {
            broker
                .admit(super::tests::request(), &sender, Instant::now())
                .unwrap();
        }
        assert_eq!(
            broker
                .admit(super::tests::request(), &sender, Instant::now())
                .unwrap_err()
                .0,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(broker.entries.len(), PENDING_LIMIT);
    }

    #[test]
    fn receipt_retention_expires_terminal_entries_but_not_pending_work() {
        let mut broker = Broker::default();
        let (sender, receiver) = crossbeam_channel::bounded(PENDING_LIMIT);
        let pending = broker.admit(request(), &sender, Instant::now()).unwrap();
        let terminal = broker.admit(request(), &sender, Instant::now()).unwrap();
        broker.complete(
            terminal.receipt_id,
            Outcome::new(State::Applied, "persisted"),
        );
        let now = Instant::now();
        broker
            .entries
            .get_mut(&terminal.receipt_id)
            .unwrap()
            .completed = Some(now - RETENTION);
        broker.prune(now);
        assert!(!broker.entries.contains_key(&terminal.receipt_id));
        assert!(broker.entries.contains_key(&pending.receipt_id));
        while receiver.try_recv().is_ok() {}
        for _ in 0..=TERMINAL_LIMIT {
            let admitted = broker.admit(request(), &sender, Instant::now()).unwrap();
            receiver.recv().unwrap();
            broker.complete(
                admitted.receipt_id,
                Outcome::new(State::Applied, "persisted"),
            );
        }
        assert_eq!(broker.entries.len(), TERMINAL_LIMIT + 1);
        assert_eq!(broker.requests.len(), TERMINAL_LIMIT + 1);
    }

    #[test]
    fn stopped_processors_and_queued_byte_pressure_never_claim_persistence() {
        let broker = Arc::new(Mutex::new(Broker::default()));
        let (sender, receiver) = crossbeam_channel::bounded(PENDING_LIMIT);
        let pending = broker
            .lock()
            .unwrap()
            .admit(request(), &sender, Instant::now())
            .unwrap();
        broker
            .lock()
            .unwrap()
            .entries
            .get_mut(&pending.receipt_id)
            .unwrap()
            .source_bytes = PENDING_BYTES;
        assert_eq!(
            broker
                .lock()
                .unwrap()
                .admit(request(), &sender, Instant::now())
                .unwrap_err()
                .0,
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(ProcessorGuard(Arc::clone(&broker)));
        assert_eq!(
            broker
                .lock()
                .unwrap()
                .test_outcome(pending.receipt_id)
                .state,
            State::ProcessorStopped
        );
        drop(receiver);
        assert_eq!(
            broker
                .lock()
                .unwrap()
                .admit(request(), &sender, Instant::now())
                .unwrap_err()
                .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(
            broker
                .lock()
                .unwrap()
                .test_outcome(pending.receipt_id)
                .may_have_changed
        );
    }
}
