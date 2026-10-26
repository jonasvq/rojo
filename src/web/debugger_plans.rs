//! Private, opt-in automatic debugger transport. Native control remains behind
//! per-connection Studio consent; this broker never executes caller code.
use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{bail, ensure, Context, Result};
use hyper::{Body, Method, Request, Response, StatusCode};
use rbx_dom_weak::types::{Ref, Variant};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    api::request_bytes,
    interface::ErrorResponse,
    source_paths,
    util::{msgpack, msgpack_ok},
};
use crate::{serve_session::ServeSession, snapshot_middleware::Middleware};

pub(super) const CAPABILITY: &str = "debuggerPlansV1";
const BODY_LIMIT: usize = 128 * 1024;
const LEASE: Duration = Duration::from_secs(5);
pub(super) type Shared = Arc<Mutex<Broker>>;

pub(super) fn limits() -> Value {
    json!({"combinedRun":true,"heldPause":false,"separatePlay":false,
        "plans":32,"points":8,"actions":16,"hits":64,"frames":12,
        "values":32,"tableDepth":2,"summaryBytes":512,"evaluations":4,
        "reportBytes":65536,"queue":64,"sourceBytes":1048576,"relayLeaseSeconds":5})
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    points: Vec<u32>,
    actions: Vec<Action>,
    max_hits: u8,
    table_depth: u8,
    #[serde(default)]
    filter: Option<Filter>,
    #[serde(default)]
    evaluations: Vec<Evaluation>,
}
#[derive(Clone, Deserialize, Serialize)]
enum Action {
    Into,
    Over,
    Out,
    Resume,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    identifier: String,
    operator: Comparison,
    value: f64,
}
#[derive(Clone, Deserialize, Serialize)]
enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Evaluation {
    identifier: String,
    #[serde(default)]
    arithmetic: Option<Arithmetic>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Arithmetic {
    operator: Operator,
    operand: f64,
}
#[derive(Clone, Deserialize, Serialize)]
enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 64
        && bytes
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !matches!(
            value,
            "and"
                | "break"
                | "continue"
                | "do"
                | "else"
                | "elseif"
                | "end"
                | "false"
                | "for"
                | "function"
                | "if"
                | "in"
                | "local"
                | "nil"
                | "not"
                | "or"
                | "repeat"
                | "return"
                | "then"
                | "true"
                | "until"
                | "while"
        )
}
impl Plan {
    fn validate(&self, lines: u32) -> Result<()> {
        ensure!(
            !self.points.is_empty() && self.points.len() <= 8,
            "Expected 1..8 points"
        );
        let mut points = self.points.clone();
        points.sort_unstable();
        points.dedup();
        ensure!(
            points.len() == self.points.len()
                && points.iter().all(|line| *line > 0 && *line <= lines),
            "Invalid or duplicate source line"
        );
        ensure!(
            !self.actions.is_empty() && self.actions.len() <= 16,
            "Expected 1..16 actions"
        );
        ensure!(
            (1..=64).contains(&self.max_hits)
                && self.table_depth <= 2
                && self.evaluations.len() <= 4,
            "Plan exceeds inspection limits"
        );
        if let Some(filter) = &self.filter {
            ensure!(
                identifier(&filter.identifier) && filter.value.is_finite(),
                "Invalid numeric filter"
            );
        }
        for eval in &self.evaluations {
            ensure!(
                identifier(&eval.identifier),
                "Invalid evaluation identifier"
            );
            if let Some(arithmetic) = &eval.arithmetic {
                ensure!(
                    arithmetic.operand.is_finite()
                        && !(matches!(arithmetic.operator, Operator::Divide)
                            && arithmetic.operand == 0.0),
                    "Invalid arithmetic operand"
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    instance_id: Ref,
    path: String,
    class_name: String,
    source_hash: String,
    line_count: u32,
    source: String,
}

fn source(session: &ServeSession, id: Ref) -> Result<Source> {
    let tree = session.tree();
    let instance = tree.get_instance(id).context("Unknown managed instance")?;
    ensure!(
        matches!(
            instance.class_name().as_str(),
            "Script" | "LocalScript" | "ModuleScript"
        ),
        "Not a native script"
    );
    use Middleware::*;
    ensure!(
        matches!(
            instance.metadata().middleware,
            Some(
                ServerScript
                    | ClientScript
                    | ModuleScript
                    | PluginScript
                    | LegacyClientScript
                    | LegacyServerScript
                    | RunContextServerScript
                    | RunContextClientScript
                    | ServerScriptDir
                    | ClientScriptDir
                    | PluginScriptDir
                    | ModuleScriptDir
            )
        ),
        "Not native Lua middleware"
    );
    let paths: Vec<_> = source_paths::paths_for(&tree, session.vfs(), id, &mut HashMap::new())?
        .into_iter()
        .filter(|path| {
            std::path::Path::new(path)
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("lua") || ext.eq_ignore_ascii_case("luau")
                })
        })
        .collect();
    ensure!(paths.len() == 1, "Expected exactly one Lua source");
    let path = &paths[0];
    let ids = tree.get_ids_at_path(std::path::Path::new(path));
    ensure!(
        ids.iter()
            .filter(|other| tree.get_instance(**other).is_some_and(|i| matches!(
                i.class_name().as_str(),
                "Script" | "LocalScript" | "ModuleScript"
            )))
            .count()
            == 1,
        "Ambiguous source binding"
    );
    let text = session.vfs().read_to_string(path)?;
    ensure!(text.len() <= 1024 * 1024, "Source exceeds 1 MiB");
    ensure!(
        matches!(instance.properties().get(&"Source".into()), Some(Variant::String(value)) if value.as_str() == text.as_str()),
        "Overridden or stale source"
    );
    Ok(Source {
        instance_id: id,
        path: path.clone(),
        class_name: instance.class_name().to_string(),
        source_hash: blake3::hash(text.as_bytes()).to_hex().to_string(),
        line_count: text.bytes().filter(|c| *c == b'\n').count() as u32 + 1,
        source: text.to_string(),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Desired {
    source: Source,
    plan: Plan,
    generation: u64,
}
struct Relay {
    id: String,
    last_seen: Instant,
    mode: Mode,
    installed: bool,
    plans: Vec<PlanState>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanState {
    instance_id: Ref,
    generation: u64,
    state: NativeState,
    points: Vec<VerifiedPoint>,
    #[serde(default)]
    error: Option<String>,
}
#[derive(Deserialize, Serialize)]
enum NativeState {
    Armed,
    Rejected,
    Completed,
    Invalidated,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VerifiedPoint {
    requested: u32,
    verified: u32,
}
#[derive(Clone, Copy, Deserialize, Serialize)]
enum Mode {
    Edit,
    Run,
    Unsupported,
}

#[derive(Default)]
pub(super) struct Broker {
    plans: HashMap<Ref, Desired>,
    relay: Option<Relay>,
    generation: u64,
    cursor: u64,
    dropped: u64,
    events: VecDeque<Value>,
}
impl Broker {
    fn refresh(&mut self, session: &ServeSession) {
        if self
            .relay
            .as_ref()
            .is_some_and(|relay| relay.last_seen.elapsed() > LEASE)
        {
            self.relay = None;
            self.plans.clear();
            self.events.clear();
        }
        let invalid: Vec<_> = self
            .plans
            .iter()
            .filter_map(|(id, desired)| {
                let valid = source(session, *id).is_ok_and(|current| {
                    current.source_hash == desired.source.source_hash
                        && current.class_name == desired.source.class_name
                        && current.path == desired.source.path
                });
                (!valid).then_some(*id)
            })
            .collect();
        for id in invalid {
            self.plans.remove(&id);
            self.events.retain(|e| e["instanceId"] != json!(id));
        }
        if let Some(relay) = &mut self.relay {
            relay.plans.retain(|state| {
                self.plans
                    .get(&state.instance_id)
                    .is_some_and(|plan| plan.generation == state.generation)
            });
        }
    }
    fn relay(&mut self, id: &str) -> Result<&mut Relay> {
        let relay = self
            .relay
            .as_mut()
            .context("Studio consent relay is disabled or expired")?;
        ensure!(relay.id == id, "Wrong relay ID");
        relay.last_seen = Instant::now();
        Ok(relay)
    }
    fn push(&mut self, mut event: Value) {
        self.cursor += 1;
        event["cursor"] = json!(self.cursor);
        if self.events.len() == 64 {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(event);
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
enum Command {
    #[serde(rename_all = "camelCase")]
    Set {
        session_id: String,
        instance_id: Ref,
        expected_source_hash: String,
        plan: Plan,
    },
    #[serde(rename_all = "camelCase")]
    Remove {
        session_id: String,
        instance_id: Ref,
        expected_source_hash: String,
    },
    #[serde(rename_all = "camelCase")]
    Disarm { session_id: String },
}
impl Command {
    fn session(&self) -> &str {
        match self {
            Self::Set { session_id, .. }
            | Self::Remove { session_id, .. }
            | Self::Disarm { session_id } => session_id,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
enum RelayCommand {
    #[serde(rename_all = "camelCase")]
    Enable {
        session_id: String,
        root_id: Ref,
        relay_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Disable {
        session_id: String,
        relay_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Status {
        session_id: String,
        relay_id: String,
        mode: Mode,
        installed: bool,
        #[serde(default)]
        plans: Vec<PlanState>,
    },
    #[serde(rename_all = "camelCase")]
    Report {
        session_id: String,
        relay_id: String,
        instance_id: Ref,
        source_hash: String,
        generation: u64,
        report: Inspection,
    },
}
impl RelayCommand {
    fn session(&self) -> &str {
        match self {
            Self::Enable { session_id, .. }
            | Self::Disable { session_id, .. }
            | Self::Status { session_id, .. }
            | Self::Report { session_id, .. } => session_id,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Inspection {
    hit: u8,
    requested_line: u32,
    verified_line: u32,
    action: Action,
    frames: Vec<Frame>,
    evaluations: Vec<Scalar>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    overflow: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Frame {
    name: String,
    line: u32,
    values: Vec<Scalar>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Scalar {
    name: String,
    kind: String,
    value: String,
    #[serde(default)]
    children: Vec<Scalar>,
}
impl Inspection {
    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=64).contains(&self.hit) && self.frames.len() <= 12 && self.evaluations.len() <= 4,
            "Invalid inspection limits"
        );
        fn scalar(v: &Scalar, depth: usize) -> bool {
            v.name.len() <= 512
                && v.kind.len() <= 64
                && v.value.len() <= 512
                && v.children.len() <= 32
                && (v.children.is_empty()
                    || depth < 2 && v.children.iter().all(|c| scalar(c, depth + 1)))
        }
        ensure!(
            self.frames.iter().all(|f| f.name.len() <= 512
                && f.values.len() <= 32
                && f.values.iter().all(|v| scalar(v, 0)))
                && self.evaluations.iter().all(|v| scalar(v, 0))
                && self.error.as_ref().is_none_or(|e| e.len() <= 512),
            "Inspection value exceeds limits"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= 65536,
            "Report exceeds 64 KiB"
        );
        Ok(())
    }
}

async fn decode<T: serde::de::DeserializeOwned>(
    request: Request<Body>,
    limit: NonZeroUsize,
) -> Result<T, Box<Response<Body>>> {
    let is_json = request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next() == Some("application/json"));
    let bytes = request_bytes(
        request,
        NonZeroUsize::new(limit.get().min(BODY_LIMIT)).unwrap(),
    )
    .await
    .map_err(|(error, status)| Box::new(msgpack(error, status)))?;
    let result: Result<T> = if is_json {
        serde_json::from_slice(&bytes).map_err(Into::into)
    } else {
        (|| {
            let mut decoder =
                rmp_serde::Deserializer::new(std::io::Cursor::new(&bytes)).with_human_readable();
            let value = T::deserialize(&mut decoder)?;
            ensure!(
                decoder.get_ref().position() == bytes.len() as u64,
                "Trailing MessagePack data"
            );
            Ok(value)
        })()
    };
    result.map_err(|e| {
        Box::new(msgpack(
            ErrorResponse::bad_request(format!("Invalid debugger body: {e}")),
            StatusCode::BAD_REQUEST,
        ))
    })
}

pub(super) async fn handle(
    session: &ServeSession,
    shared: &Shared,
    request: Request<Body>,
    limit: NonZeroUsize,
) -> Response<Body> {
    let method = request.method().clone();
    let route = request.uri().path().to_owned();
    let query: HashMap<String, String> = request
        .uri()
        .query()
        .unwrap_or("")
        .split('&')
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let command = if method == Method::POST && route == "/api/debugger/plans" {
        match decode::<Command>(request, limit).await {
            Ok(v) => Some(Ok(v)),
            Err(r) => return *r,
        }
    } else if method == Method::POST && route == "/api/debugger/relay" {
        match decode::<RelayCommand>(request, limit).await {
            Ok(v) => Some(Err(v)),
            Err(r) => return *r,
        }
    } else {
        None
    };
    let result = (|| -> Result<Value> {
        let mut broker = shared.lock().unwrap_or_else(|e| e.into_inner());
        broker.refresh(session);
        let expected = session.session_id().to_string();
        let supplied = match &command {
            Some(Ok(c)) => Some(c.session()),
            Some(Err(c)) => Some(c.session()),
            None => query.get("sessionId").map(String::as_str),
        };
        ensure!(supplied == Some(expected.as_str()), "Wrong session ID");
        match command {
            Some(Ok(Command::Set {
                instance_id,
                expected_source_hash,
                plan,
                ..
            })) => {
                let source = source(session, instance_id)?;
                ensure!(
                    source.source_hash == expected_source_hash,
                    "Stale source hash"
                );
                plan.validate(source.line_count)?;
                ensure!(
                    broker.plans.contains_key(&instance_id) || broker.plans.len() < 32,
                    "Plan limit reached"
                );
                broker.generation += 1;
                let generation = broker.generation;
                broker
                    .events
                    .retain(|e| e["instanceId"] != json!(instance_id));
                broker.plans.insert(
                    instance_id,
                    Desired {
                        source,
                        plan,
                        generation,
                    },
                );
                Ok(json!({"generation":generation}))
            }
            Some(Ok(Command::Remove {
                instance_id,
                expected_source_hash,
                ..
            })) => {
                ensure!(
                    broker
                        .plans
                        .get(&instance_id)
                        .is_some_and(|p| p.source.source_hash == expected_source_hash),
                    "Stale or missing plan"
                );
                broker.plans.remove(&instance_id);
                broker
                    .events
                    .retain(|e| e["instanceId"] != json!(instance_id));
                Ok(json!({}))
            }
            Some(Ok(Command::Disarm { .. })) => {
                broker.plans.clear();
                broker.events.clear();
                broker.relay = None;
                Ok(json!({}))
            }
            Some(Err(RelayCommand::Enable {
                root_id, relay_id, ..
            })) => {
                ensure!(
                    root_id == session.tree().get_root_id(),
                    "Wrong accepted root"
                );
                ensure!(
                    relay_id.len() == 36 && uuid::Uuid::parse_str(&relay_id).is_ok(),
                    "Invalid relay ID"
                );
                ensure!(
                    broker.relay.as_ref().is_none_or(|r| r.id == relay_id),
                    "Another Studio relay owns this session"
                );
                broker.relay = Some(Relay {
                    id: relay_id,
                    last_seen: Instant::now(),
                    mode: Mode::Edit,
                    installed: false,
                    plans: Vec::new(),
                });
                Ok(json!({}))
            }
            Some(Err(RelayCommand::Disable { relay_id, .. })) => {
                broker.relay(&relay_id)?;
                broker.relay = None;
                broker.plans.clear();
                broker.events.clear();
                Ok(json!({}))
            }
            Some(Err(RelayCommand::Status {
                relay_id,
                mode,
                installed,
                plans,
                ..
            })) => {
                ensure!(
                    !installed || matches!(mode, Mode::Run),
                    "Callback installed outside combined Run"
                );
                ensure!(plans.len() <= 32, "Too many native plan states");
                let mut seen = std::collections::HashSet::new();
                for state in &plans {
                    let desired = broker
                        .plans
                        .get(&state.instance_id)
                        .context("Unknown native plan")?;
                    ensure!(
                        seen.insert(state.instance_id)
                            && desired.generation == state.generation
                            && state.points.len() <= 8
                            && state.error.as_ref().is_none_or(|e| e.len() <= 512),
                        "Invalid native plan state"
                    );
                    ensure!(
                        state
                            .points
                            .iter()
                            .all(|p| desired.plan.points.contains(&p.requested)
                                && p.verified > 0
                                && p.verified <= desired.source.line_count),
                        "Invalid verified native point"
                    );
                    ensure!(
                        !matches!(state.state, NativeState::Armed)
                            || installed && !state.points.is_empty(),
                        "Armed state requires installed points"
                    );
                }
                let r = broker.relay(&relay_id)?;
                let exited = matches!(r.mode, Mode::Run) && !matches!(mode, Mode::Run);
                r.mode = mode;
                r.installed = installed;
                r.plans = plans;
                if exited {
                    broker.plans.clear();
                    broker.events.clear();
                }
                Ok(json!({}))
            }
            Some(Err(RelayCommand::Report {
                relay_id,
                instance_id,
                source_hash,
                generation,
                report,
                ..
            })) => {
                ensure!(
                    matches!(broker.relay(&relay_id)?.mode, Mode::Run),
                    "Relay is not in combined Run"
                );
                let desired = broker.plans.get(&instance_id).context("Missing plan")?;
                ensure!(
                    desired.generation == generation && desired.source.source_hash == source_hash,
                    "Stale report"
                );
                report.validate()?;
                ensure!(
                    report.hit <= desired.plan.max_hits
                        && desired.plan.points.contains(&report.requested_line)
                        && report.verified_line > 0
                        && report.verified_line <= desired.source.line_count,
                    "Inspection does not match the accepted plan"
                );
                broker.push(json!({"instanceId":instance_id,"sourceHash":source_hash,"generation":generation,"report":report}));
                Ok(json!({}))
            }
            None if method == Method::GET => match route.as_str() {
                "/api/debugger/sources" => {
                    let ids: Vec<_> = query
                        .get("ids")
                        .context("Missing ids")?
                        .split(',')
                        .collect();
                    ensure!(!ids.is_empty() && ids.len() <= 32, "Expected 1..32 IDs");
                    let mut sources = Vec::new();
                    for id in ids {
                        let parsed: Ref = id.parse().context("Invalid instance ID")?;
                        sources.push(match source(session, parsed) {
                            Ok(s) => json!(s),
                            Err(e) => json!({"instanceId":parsed,"error":e.to_string()}),
                        });
                    }
                    Ok(json!({"sources":sources}))
                }
                "/api/debugger/status" => Ok(
                    json!({"relay":broker.relay.as_ref().map(|r|json!({"mode":r.mode,"installed":r.installed,"plans":r.plans})),"plans":broker.plans.values().collect::<Vec<_>>(),"cursor":broker.cursor,"dropped":broker.dropped}),
                ),
                "/api/debugger/events" => {
                    let cursor = query
                        .get("cursor")
                        .map(|c| c.parse::<u64>())
                        .transpose()?
                        .unwrap_or(0);
                    ensure!(cursor <= broker.cursor, "Future cursor");
                    Ok(
                        json!({"cursor":broker.cursor,"dropped":broker.dropped,"events":broker.events.iter().filter(|e|e["cursor"].as_u64().is_some_and(|c|c>cursor)).collect::<Vec<_>>() }),
                    )
                }
                "/api/debugger/relay" => {
                    broker.relay(query.get("relayId").context("Missing relay ID")?)?;
                    Ok(
                        json!({"rootId":session.tree().get_root_id(),"plans":broker.plans.values().collect::<Vec<_>>()}),
                    )
                }
                _ => bail!("Unknown debugger route"),
            },
            None => bail!("Unknown debugger route or method"),
        }
    })();
    match result {
        Ok(mut value) => {
            value["sessionId"] = json!(session.session_id());
            msgpack_ok(value)
        }
        Err(error) => msgpack(
            ErrorResponse::bad_request(error.to_string()),
            StatusCode::BAD_REQUEST,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::super::util::deserialize_msgpack;
    use super::*;
    use memofs::{InMemoryFs, Vfs, VfsSnapshot};

    fn session() -> ServeSession {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/debug",
            VfsSnapshot::dir([
                (
                    "default.project.json",
                    VfsSnapshot::file(
                        r#"{"name":"Debug","tree":{"$className":"Folder","$path":"src"}}"#,
                    ),
                ),
                (
                    "src",
                    VfsSnapshot::dir([
                        (
                            "target.server.luau",
                            VfsSnapshot::file("local localCounter = 7\nprint(localCounter)\n"),
                        ),
                        ("generated.txt", VfsSnapshot::file("not native source")),
                    ]),
                ),
            ]),
        )
        .unwrap();
        let vfs = Vfs::new(fs);
        vfs.set_watch_enabled(false);
        ServeSession::new(vfs, "/debug/default.project.json").unwrap()
    }
    fn plan() -> Value {
        json!({"points":[1,2],"actions":["Into","Over","Out","Resume"],"maxHits":8,"tableDepth":2,"filter":{"identifier":"localCounter","operator":"Ge","value":7},"evaluations":[{"identifier":"localCounter","arithmetic":{"operator":"Add","operand":1}}]})
    }
    fn target(session: &ServeSession) -> Ref {
        session
            .tree()
            .inner()
            .descendants()
            .find(|i| i.name == "target")
            .unwrap()
            .referent()
    }

    #[test]
    fn ambiguous_and_overridden_native_sources_are_ineligible() {
        for project in [
            r#"{"name":"Debug","tree":{"$className":"Folder","A":{"$path":"source.server.luau"},"B":{"$path":"source.server.luau"}}}"#,
            r#"{"name":"Debug","tree":{"$className":"Folder","A":{"$path":"source.server.luau","$properties":{"Source":"print('override')"}}}}"#,
        ] {
            let mut fs = InMemoryFs::new();
            fs.load_snapshot(
                "/ambiguous",
                VfsSnapshot::dir([
                    ("default.project.json", VfsSnapshot::file(project)),
                    ("source.server.luau", VfsSnapshot::file("print('native')")),
                ]),
            )
            .unwrap();
            let vfs = Vfs::new(fs);
            vfs.set_watch_enabled(false);
            let session = ServeSession::new(vfs, "/ambiguous/default.project.json").unwrap();
            let id = session
                .tree()
                .inner()
                .descendants()
                .find(|i| i.name == "A")
                .unwrap()
                .referent();
            assert!(source(&session, id).is_err());
        }
    }
    async fn request(
        session: &ServeSession,
        broker: &Shared,
        route: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = match body {
            Some(value) => Request::post(route)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
            None => Request::get(route).body(Body::empty()).unwrap(),
        };
        let response = handle(
            session,
            broker,
            request,
            NonZeroUsize::new(BODY_LIMIT).unwrap(),
        )
        .await;
        let status = response.status();
        (
            status,
            deserialize_msgpack(&hyper::body::to_bytes(response.into_body()).await.unwrap())
                .unwrap(),
        )
    }
    #[test]
    fn typed_grammar_rejects_code_and_unbounded_inspection() {
        let valid: Plan = serde_json::from_value(plan()).unwrap();
        valid.validate(3).unwrap();
        for name in [
            "",
            "end",
            "return",
            "x.y",
            "x[1]",
            "f()",
            "x;print(1)",
            "é",
            "1x",
        ] {
            let mut value = plan();
            value["evaluations"][0]["identifier"] = json!(name);
            assert!(
                serde_json::from_value::<Plan>(value)
                    .unwrap()
                    .validate(3)
                    .is_err(),
                "{name}"
            );
        }
        for (field, value) in [
            ("points", json!([0])),
            ("points", json!([1, 1])),
            ("points", json!([4])),
            ("points", json!([])),
            ("actions", json!(["Pause"])),
            ("maxHits", json!(65)),
            ("tableDepth", json!(3)),
            ("condition", json!("print(1)")),
        ] {
            let mut p = plan();
            p[field] = value;
            assert!(
                serde_json::from_value::<Plan>(p)
                    .and_then(|p| p.validate(3).map_err(serde::de::Error::custom))
                    .is_err(),
                "{field}"
            );
        }
        let mut p = plan();
        p["evaluations"][0]["arithmetic"] = json!({"operator":"Divide","operand":0});
        assert!(serde_json::from_value::<Plan>(p)
            .unwrap()
            .validate(3)
            .is_err());
        let mut p: Plan = serde_json::from_value(plan()).unwrap();
        p.filter.as_mut().unwrap().value = f64::INFINITY;
        assert!(p.validate(3).is_err());
    }
    #[tokio::test]
    async fn source_versions_relay_consent_reports_and_revocation() {
        let session = session();
        let broker = Shared::default();
        let id = target(&session);
        let sid = session.session_id().to_string();
        let original = source(&session, id).unwrap();
        assert_eq!(original.line_count, 3);
        assert!(original.path.ends_with("target.server.luau"));
        let generated = session
            .tree()
            .inner()
            .descendants()
            .find(|i| i.name == "generated")
            .unwrap()
            .referent();
        assert!(source(&session, generated).is_err());
        let set = json!({"operation":"set","sessionId":sid,"instanceId":id,"expectedSourceHash":original.source_hash,"plan":plan()});
        let (status, result) =
            request(&session, &broker, "/api/debugger/plans", Some(set.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let generation = result["generation"].clone();
        let relay_id = uuid::Uuid::new_v4().to_string();
        let enable = json!({"operation":"enable","sessionId":sid,"rootId":session.tree().get_root_id(),"relayId":relay_id});
        assert_eq!(
            request(
                &session,
                &broker,
                "/api/debugger/relay",
                Some(enable.clone())
            )
            .await
            .0,
            StatusCode::OK
        );
        let mut another = enable;
        another["relayId"] = json!(uuid::Uuid::new_v4().to_string());
        assert_eq!(
            request(&session, &broker, "/api/debugger/relay", Some(another))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let status_body = json!({"operation":"status","sessionId":sid,"relayId":relay_id,"mode":"Run","installed":true});
        assert_eq!(
            request(
                &session,
                &broker,
                "/api/debugger/relay",
                Some(status_body.clone())
            )
            .await
            .0,
            StatusCode::OK
        );
        let report = json!({"operation":"report","sessionId":sid,"relayId":relay_id,"instanceId":id,"sourceHash":original.source_hash,"generation":generation,"report":{"hit":1,"requestedLine":1,"verifiedLine":2,"action":"Into","frames":[{"name":"target","line":2,"values":[{"name":"localCounter","kind":"number","value":"7"}]}],"evaluations":[{"name":"localCounter+1","kind":"number","value":"8"}]}});
        assert_eq!(
            request(
                &session,
                &broker,
                "/api/debugger/relay",
                Some(report.clone())
            )
            .await
            .0,
            StatusCode::OK
        );
        let route = format!("/api/debugger/events?sessionId={sid}&cursor=0");
        let captured = request(&session, &broker, &route, None).await.1;
        assert_eq!(
            captured["events"][0]["report"]["evaluations"][0]["value"],
            "8"
        );
        assert_eq!(
            request(&session, &broker, &route, None).await.1,
            captured,
            "reads must not mutate reports"
        );
        let mut wrong = report.clone();
        wrong["generation"] = json!(999);
        assert_eq!(
            request(&session, &broker, "/api/debugger/relay", Some(wrong))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let mut invalid = status_body;
        invalid["mode"] = json!("Edit");
        assert_eq!(
            request(&session, &broker, "/api/debugger/relay", Some(invalid))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let mut stale = set;
        stale["expectedSourceHash"] = json!("old");
        assert_eq!(
            request(&session, &broker, "/api/debugger/plans", Some(stale))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let mut oversized = report;
        for (field, value) in [
            ("hit", 9),
            ("verifiedLine", 0),
            ("verifiedLine", 4),
            ("requestedLine", 3),
        ] {
            let mut invalid = oversized.clone();
            invalid["report"][field] = json!(value);
            assert_eq!(
                request(&session, &broker, "/api/debugger/relay", Some(invalid))
                    .await
                    .0,
                StatusCode::BAD_REQUEST
            );
        }
        oversized["report"]["evaluations"][0]["value"] = json!("x".repeat(513));
        assert_eq!(
            request(&session, &broker, "/api/debugger/relay", Some(oversized))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            request(
                &session,
                &broker,
                "/api/debugger/plans",
                Some(json!({"operation":"disarm","sessionId":sid}))
            )
            .await
            .0,
            StatusCode::OK
        );
        let state = broker.lock().unwrap();
        assert!(state.relay.is_none() && state.plans.is_empty() && state.events.is_empty());
    }
    #[tokio::test]
    async fn stale_sources_expired_relays_and_overflow_are_bounded() {
        let session = session();
        let broker = Shared::default();
        let id = target(&session);
        let source = source(&session, id).unwrap();
        {
            let mut state = broker.lock().unwrap();
            state.plans.insert(
                id,
                Desired {
                    source: source.clone(),
                    plan: serde_json::from_value(plan()).unwrap(),
                    generation: 1,
                },
            );
            state.relay = Some(Relay {
                id: "expired".into(),
                last_seen: Instant::now() - LEASE - Duration::from_secs(1),
                mode: Mode::Run,
                installed: true,
                plans: Vec::new(),
            });
            for _ in 0..70 {
                state.push(json!({"instanceId":id,"sourceHash":source.source_hash}));
            }
            assert_eq!(state.events.len(), 64);
            assert_eq!(state.dropped, 6);
        }
        // Direct disk/VFS changes invalidate even before a tree snapshot catches up.
        session
            .vfs()
            .write("/debug/src/target.server.luau", b"print('changed')")
            .unwrap();
        let route = format!("/api/debugger/status?sessionId={}", session.session_id());
        assert_eq!(
            request(&session, &broker, &route, None).await.0,
            StatusCode::OK
        );
        let state = broker.lock().unwrap();
        assert!(state.plans.is_empty() && state.events.is_empty() && state.relay.is_none());
    }
    #[tokio::test]
    async fn strict_session_shape_body_and_source_count_limits() {
        let session = session();
        let broker = Shared::default();
        let sid = session.session_id().to_string();
        for body in [
            json!({"operation":"disarm","sessionId":"wrong"}),
            json!({"operation":"disarm","sessionId":sid,"code":"print(1)"}),
            json!({"operation":"play","sessionId":sid}),
        ] {
            assert_eq!(
                request(&session, &broker, "/api/debugger/plans", Some(body))
                    .await
                    .0,
                StatusCode::BAD_REQUEST
            );
        }
        let ids = vec![target(&session).to_string(); 33].join(",");
        assert_eq!(
            request(
                &session,
                &broker,
                &format!("/api/debugger/sources?sessionId={sid}&ids={ids}"),
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        let request = Request::post("/api/debugger/plans")
            .body(Body::from(vec![0; BODY_LIMIT + 1]))
            .unwrap();
        assert_eq!(
            handle(
                &session,
                &broker,
                request,
                NonZeroUsize::new(BODY_LIMIT).unwrap()
            )
            .await
            .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        let mut trailing =
            super::super::util::serialize_msgpack(json!({"operation":"disarm","sessionId":sid}))
                .unwrap();
        trailing.push(0xc0);
        let request = Request::post("/api/debugger/plans")
            .body(Body::from(trailing))
            .unwrap();
        assert_eq!(
            handle(
                &session,
                &broker,
                request,
                NonZeroUsize::new(BODY_LIMIT).unwrap()
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
}
