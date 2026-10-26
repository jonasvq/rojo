//! Replay with ROJO_CHURN_SEED, ROJO_CHURN_OPERATIONS, and ROJO_CHURN_BINARY.
//! Each run retains the project, operation transcript, and server log in target/churn.
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use librojo::{snapshot_from_vfs, web_api::ReadResponse, InstanceContext, RojoTree};
use memofs::Vfs;
use rbx_dom_weak::types::Ref;
use serde_json::{json, Value};

use crate::rojo_test::{io_util::ROJO_PATH, serve_util::deserialize_msgpack};

const DEADLINE: Duration = Duration::from_secs(10);

struct Server {
    child: Child,
    url: String,
    client: reqwest::blocking::Client,
}

impl Server {
    fn start(project: &Path, artifacts: &Path) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let log = File::create(artifacts.join("server.log")).unwrap();
        let binary = std::env::var_os("ROJO_CHURN_BINARY").unwrap_or_else(|| ROJO_PATH.into());
        let child = Command::new(binary)
            .arg("serve")
            .arg(project)
            .args(["--port", &port.to_string(), "-vvv"])
            .env("RUST_BACKTRACE", "1")
            .env("RUST_LOG", "warn,memofs=trace")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(1))
                .connect_timeout(Duration::from_millis(200))
                .build()
                .unwrap(),
        }
    }

    fn read(&mut self) -> anyhow::Result<Value> {
        if let Some(status) = self.child.try_wait()? {
            anyhow::bail!("server exited: {status}");
        }
        let body = self
            .client
            .get(format!("{}/api/rojo", self.url))
            .send()?
            .error_for_status()?
            .bytes()?;
        let info: librojo::web_api::ServerInfoResponse = deserialize_msgpack(&body)?;
        let body = self
            .client
            .get(format!("{}/api/read/{}", self.url, info.root_instance_id))
            .send()?
            .error_for_status()?
            .bytes()?;
        let response: ReadResponse = deserialize_msgpack(&body)?;
        let nodes = response
            .instances
            .into_iter()
            .map(|(id, node)| {
                (
                    id,
                    Node {
                        name: node.name.into_owned(),
                        class: node.class_name.to_string(),
                        properties: serde_json::to_value(node.properties).unwrap(),
                        children: node.children.into_owned(),
                    },
                )
            })
            .collect();
        Ok(normalize(info.root_instance_id, &nodes))
    }

    fn converge(&mut self, expected: &Value, artifacts: &Path) {
        let started = Instant::now();
        let mut last = None;
        let mut error = None;
        while started.elapsed() < DEADLINE {
            match self.read() {
                Ok(actual) if actual == *expected => return,
                Ok(actual) => last = Some(actual),
                Err(err) => {
                    error = Some(err.to_string());
                    if self.child.try_wait().unwrap().is_some() {
                        break;
                    }
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
        fs::write(
            artifacts.join("expected.json"),
            serde_json::to_vec_pretty(expected).unwrap(),
        )
        .unwrap();
        fs::write(
            artifacts.join("actual.json"),
            serde_json::to_vec_pretty(&last).unwrap(),
        )
        .unwrap();
        panic!(
            "churn failed: error={error:?}; artifacts={}",
            artifacts.display()
        );
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Node {
    name: String,
    class: String,
    properties: Value,
    children: Vec<Ref>,
}

fn normalize(id: Ref, nodes: &HashMap<Ref, Node>) -> Value {
    let node = &nodes[&id];
    // Filesystem sibling insertion order can differ after incremental edits.
    // Compare a multiset: duplicates must remain visible instead of being deduplicated.
    let mut children: Vec<_> = node
        .children
        .iter()
        .map(|id| normalize(*id, nodes))
        .collect();
    children.sort_by_cached_key(Value::to_string);
    json!({
        "name": node.name,
        "class": node.class,
        "properties": node.properties,
        "children": children,
    })
}

fn fresh_snapshot(project: &Path) -> Value {
    let vfs = Vfs::new_default().unwrap();
    vfs.set_watch_enabled(false);
    let project = vfs.canonicalize(project).unwrap();
    let snapshot = snapshot_from_vfs(&InstanceContext::default(), &vfs, &project)
        .unwrap()
        .unwrap();
    let tree = RojoTree::new(snapshot);
    let nodes = tree
        .descendants(tree.get_root_id())
        .map(|node| {
            (
                node.id(),
                Node {
                    name: node.name().to_owned(),
                    class: node.class_name().to_string(),
                    properties: serde_json::to_value(node.properties()).unwrap(),
                    children: node.children().to_vec(),
                },
            )
        })
        .collect();
    normalize(tree.get_root_id(), &nodes)
}

#[test]
fn memofs_trace_identifies_operations_without_file_contents() {
    let artifacts = tempfile::tempdir().unwrap();
    let src = artifacts.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("secret.luau"), "return 'CHURN_PRIVATE_SOURCE'").unwrap();
    let project = artifacts.path().join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"Trace","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    let mut server = Server::start(&project, artifacts.path());
    server.converge(&fresh_snapshot(&project), artifacts.path());
    let log = fs::read_to_string(artifacts.path().join("server.log")).unwrap();
    assert!(
        log.contains("memofs"),
        "missing memofs operation diagnostics: {log}"
    );
    assert!(log.contains("read_to_string") && log.contains("secret.luau"));
    // Other Rojo trace targets can include snapshots and source; inspect only memofs.
    for line in log.lines().filter(|line| line.contains("memofs")) {
        assert!(!line.contains("CHURN_PRIVATE_SOURCE"));
    }
}

#[test]
fn watcher_detects_recreated_mapped_root_and_later_files() {
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/churn")
        .join(format!("root-recreation-{run}"));
    let src = artifacts.join("src");
    fs::create_dir_all(&src).unwrap();
    let project = artifacts.join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"Root","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    fs::write(src.join("before.luau"), "return 1").unwrap();
    let mut server = Server::start(&project, &artifacts);
    server.converge(&fresh_snapshot(&project), &artifacts);
    remove(&src).unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
    retry_fs(|| fs::create_dir(&src)).unwrap();
    fs::write(src.join("after.luau"), "return 2").unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
}

#[test]
fn folder_move_then_descendant_edit_and_old_path_recreation_converges() {
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/churn")
        .join(format!("folder-move-{run}"));
    let src = artifacts.join("src");
    let old = src.join("old");
    let moved = src.join("moved");
    fs::create_dir_all(old.join("deep")).unwrap();
    fs::write(old.join("deep/init.luau"), "return 1").unwrap();
    let project = artifacts.join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"Move","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    let mut server = Server::start(&project, &artifacts);
    server.converge(&fresh_snapshot(&project), &artifacts);
    fs::rename(&old, &moved).unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
    fs::write(moved.join("deep/init.luau"), "return 2").unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
    fs::create_dir_all(old.join("deep")).unwrap();
    fs::write(old.join("deep/init.luau"), "return 3").unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
}

#[test]
fn watcher_detects_atomic_project_replacement_and_subsequent_edits() {
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/churn")
        .join(format!("project-save-{run}"));
    fs::create_dir_all(artifacts.join("src")).unwrap();
    let project = artifacts.join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"Before","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    let mut server = Server::start(&project, &artifacts);
    server.converge(&fresh_snapshot(&project), &artifacts);
    let temporary = artifacts.join("save.tmp");
    fs::write(
        &temporary,
        r#"{"name":"After","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    fs::rename(&temporary, &project).unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
    fs::write(
        &project,
        r#"{"name":"Later","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
}

#[test]
fn occupied_port_reports_an_actionable_error_without_panic() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"Port","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    let mut server = Server::start(&project, dir.path());
    server.converge(&fresh_snapshot(&project), dir.path());
    let log_path = dir.path().join("collision.log");
    let log = File::create(&log_path).unwrap();
    let mut collision = Command::new(ROJO_PATH)
        .arg("serve")
        .arg(&project)
        .args(["--port", server.url.rsplit(':').next().unwrap()])
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = collision.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() >= DEADLINE {
            collision.kill().unwrap();
            collision.wait().unwrap();
            panic!("second server did not report the port conflict");
        }
        thread::sleep(Duration::from_millis(25));
    };
    assert!(!status.success());
    let log = fs::read_to_string(log_path).unwrap();
    assert!(
        log.contains("Could not start the Rojo server") && log.contains("--port"),
        "{log}"
    );
    assert!(!log.contains("Rojo crashed!"), "{log}");
    server.converge(&fresh_snapshot(&project), dir.path());
}

#[test]
fn delayed_websocket_replays_add_write_remove_after_the_file_is_gone() {
    use hyper_tungstenite::tungstenite::{client, Message};
    use librojo::web_api::{SocketPacket, SocketPacketBody};
    use std::net::TcpStream;

    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/churn")
        .join(format!("delayed-websocket-{run}"));
    let src = artifacts.join("src");
    fs::create_dir_all(&src).unwrap();
    let project = artifacts.join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"History","tree":{"$className":"Folder","$path":"src"}}"#,
    )
    .unwrap();
    let mut server = Server::start(&project, &artifacts);
    server.converge(&fresh_snapshot(&project), &artifacts);
    let script = src.join("temporary.luau");
    for source in ["return 1", "return 2"] {
        fs::write(&script, source).unwrap();
        server.converge(&fresh_snapshot(&project), &artifacts);
    }
    fs::remove_file(&script).unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);

    let address = server.url.strip_prefix("http://").unwrap().parse().unwrap();
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let (mut socket, _) = client(format!("ws://{address}/api/socket/0"), stream).unwrap();
    let started = Instant::now();
    let mut created = None;
    let mut removals = Vec::new();
    let mut saw_update = false;
    loop {
        assert!(
            started.elapsed() < DEADLINE,
            "incomplete replay; artifacts={}",
            artifacts.display()
        );
        let message = match socket.read() {
            Ok(Message::Binary(bytes)) => bytes,
            Ok(_) => continue,
            Err(hyper_tungstenite::tungstenite::Error::Io(err))
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(err) => panic!(
                "history replay failed: {err}; artifacts={}",
                artifacts.display()
            ),
        };
        let packet: SocketPacket = deserialize_msgpack(&message).unwrap();
        let SocketPacketBody::Messages(body) = packet.body;
        for patch in body.messages {
            for (id, instance) in patch.added {
                if instance.name == "temporary" {
                    assert!(created.replace(id).is_none(), "addition was duplicated");
                    assert_eq!(
                        instance.properties[&rbx_dom_weak::ustr("Source")].as_ref(),
                        &rbx_dom_weak::types::Variant::String("return 1".into())
                    );
                }
            }
            for update in patch.updated {
                saw_update |= update.changed_properties.get(&rbx_dom_weak::ustr("Source"))
                    == Some(&Some(rbx_dom_weak::types::Variant::String(
                        "return 2".into(),
                    )));
            }
            removals.extend(patch.removed);
        }
        if created.is_some_and(|id| removals.contains(&id)) {
            assert!(saw_update);
            break;
        }
    }
    server.converge(&fresh_snapshot(&project), &artifacts);
}

#[test]
fn delayed_property_removals_are_negotiated_per_client_and_survive_later_updates() {
    use hyper_tungstenite::tungstenite::{client, Message};
    use std::net::TcpStream;

    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/churn")
        .join(format!("property-replay-{run}"));
    fs::create_dir_all(&artifacts).unwrap();
    let project = artifacts.join("default.project.json");
    fs::write(
        &project,
        r#"{"name":"Replay","tree":{"$className":"Part","$properties":{"Transparency":0.5}}}"#,
    )
    .unwrap();
    let mut server = Server::start(&project, &artifacts);
    server.converge(&fresh_snapshot(&project), &artifacts);
    fs::write(
        &project,
        r#"{"name":"Replay","tree":{"$className":"Part"}}"#,
    )
    .unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);
    fs::write(
        &project,
        r#"{"name":"Replay","tree":{"$className":"Part","$properties":{"Transparency":0.25}}}"#,
    )
    .unwrap();
    server.converge(&fresh_snapshot(&project), &artifacts);

    for negotiated in [false, true] {
        let address = server.url.strip_prefix("http://").unwrap().parse().unwrap();
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let query = if negotiated {
            "?propertyRemovalsV1=true"
        } else {
            ""
        };
        let (mut socket, _) =
            client(format!("ws://{address}/api/socket/0{query}"), stream).unwrap();
        let started = Instant::now();
        let mut removed = false;
        let mut restored = false;
        while !removed || !restored {
            assert!(
                started.elapsed() < DEADLINE,
                "incomplete replay; negotiated={negotiated}; artifacts={}",
                artifacts.display()
            );
            let bytes = match socket.read() {
                Ok(Message::Binary(bytes)) => bytes,
                Ok(_) => continue,
                Err(hyper_tungstenite::tungstenite::Error::Io(err))
                    if matches!(
                        err.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue
                }
                Err(err) => panic!(
                    "property replay failed: {err}; artifacts={}",
                    artifacts.display()
                ),
            };
            let packet: Value = deserialize_msgpack(&bytes).unwrap();
            for patch in packet["body"]["messages"].as_array().unwrap() {
                for update in patch["updated"].as_array().unwrap() {
                    if update["changedProperties"].get("Transparency") == Some(&Value::Null) {
                        assert!(!removed, "property removal was duplicated");
                        removed = true;
                        if negotiated {
                            assert_eq!(update["removedProperties"], json!(["Transparency"]));
                        } else {
                            assert!(update.get("removedProperties").is_none());
                        }
                    }
                    if update["changedProperties"]["Transparency"] == json!({"Float32":0.25}) {
                        assert!(removed, "update overtook the property removal");
                        restored = true;
                    }
                }
            }
        }
        socket.close(None).unwrap();
    }
    server.converge(&fresh_snapshot(&project), &artifacts);
}

#[test]
fn tweeninfo_replay_negotiates_added_and_updated_attributes_per_client() {
    use hyper_tungstenite::tungstenite::{client, Message};
    use rbx_dom_weak::{
        types::{Attributes, Variant},
        ustr,
    };
    use std::net::TcpStream;

    let artifacts = tempfile::tempdir().unwrap();
    let source = artifacts.path().join("src");
    fs::create_dir(&source).unwrap();
    let project = artifacts.path().join("default.project.json");
    fs::write(&project, r#"{"name":"Theme","tree":{"$path":"src"}}"#).unwrap();
    let mut server = Server::start(&project, artifacts.path());
    server.converge(&fresh_snapshot(&project), artifacts.path());
    let xml = r#"<roblox version="4"><Item class="StyleSheet" referent="R"><Properties><string name="Name">Theme</string><BinaryString name="AttributesSerialize">AgAAAAsAAABBY2NlbnRDb2xvcgIFAAAAJFBpbmsLAAAATW90aW9uSG92ZXIkCtcjPAAAAAAAAAAAAwAAAAAAAAAA</BinaryString></Properties></Item></roblox>"#;
    let mut model = rbx_xml::from_reader(xml.as_bytes(), rbx_xml::DecodeOptions::new()).unwrap();
    let id = model.root().children()[0];
    let info = server
        .client
        .get(format!("{}/api/rojo", server.url))
        .send()
        .unwrap()
        .bytes()
        .unwrap();
    let info: librojo::web_api::ServerInfoResponse = deserialize_msgpack(&info).unwrap();
    for time in [0.01_f32, 0.2] {
        let Variant::Attributes(attrs) = model
            .get_by_ref_mut(id)
            .unwrap()
            .properties
            .get_mut(&ustr("Attributes"))
            .unwrap()
        else {
            panic!("Missing attributes")
        };
        let Some(Variant::TweenInfo(mut tween)) = attrs.get("MotionHover").cloned() else {
            panic!("Missing tween")
        };
        tween.time = time;
        attrs.insert("MotionHover".into(), tween.into());
        let mut bytes = Vec::new();
        rbx_xml::to_writer(&mut bytes, &model, &[id], rbx_xml::EncodeOptions::new()).unwrap();
        fs::write(source.join("Theme.rbxmx"), bytes).unwrap();
        let deadline = Instant::now() + DEADLINE;
        loop {
            assert!(Instant::now() < deadline, "TweenInfo update stalled");
            let bytes = server
                .client
                .get(format!(
                    "{}/api/read/{}?attributeTweenInfoV1=true",
                    server.url, info.root_instance_id
                ))
                .send()
                .unwrap()
                .bytes()
                .unwrap();
            let value: Value = deserialize_msgpack(&bytes).unwrap();
            if value["instances"]
                .as_object()
                .unwrap()
                .values()
                .any(|instance| {
                    instance["Properties"]["Attributes"]["Attributes"]["MotionHover"]["TweenInfo"]
                        ["time"]
                        .as_f64()
                        .is_some_and(|actual| (actual - f64::from(time)).abs() < 0.00001)
                })
            {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    for negotiated in [false, true] {
        let address = server.url.strip_prefix("http://").unwrap().parse().unwrap();
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let query = if negotiated {
            "?propertyRemovalsV1=true&attributeTweenInfoV1=true"
        } else {
            ""
        };
        let (mut socket, _) =
            client(format!("ws://{address}/api/socket/0{query}"), stream).unwrap();
        let deadline = Instant::now() + DEADLINE;
        let (mut added, mut updated) = (false, false);
        while !added || !updated {
            assert!(Instant::now() < deadline, "Incomplete attribute replay");
            let Message::Binary(bytes) = socket.read().unwrap() else {
                continue;
            };
            let packet: Value = deserialize_msgpack(&bytes).unwrap();
            for message in packet["body"]["messages"].as_array().unwrap() {
                let added_values =
                    message["added"]
                        .as_object()
                        .unwrap()
                        .values()
                        .filter_map(|instance| {
                            instance["Properties"]
                                .get("Attributes")
                                .map(|value| (value, true))
                        });
                let updated_values =
                    message["updated"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|update| {
                            update["changedProperties"]
                                .get("Attributes")
                                .map(|value| (value, false))
                        });
                for (encoded, is_addition) in added_values.chain(updated_values) {
                    let attributes: Variant = serde_json::from_value(encoded.clone()).unwrap();
                    let attributes = match (negotiated, attributes) {
                        (true, Variant::Attributes(attributes)) => attributes,
                        (false, Variant::BinaryString(bytes)) => {
                            Attributes::from_reader(AsRef::<[u8]>::as_ref(&bytes)).unwrap()
                        }
                        other => panic!("Wrong negotiated encoding: {other:?}"),
                    };
                    let Some(Variant::TweenInfo(tween)) = attributes.get("MotionHover") else {
                        panic!("TweenInfo lost")
                    };
                    assert!(attributes.get("AccentColor").is_some());
                    assert!((tween.time - if is_addition { 0.01 } else { 0.2 }).abs() < 0.00001);
                    if is_addition {
                        added = true;
                    } else {
                        updated = true;
                    }
                }
            }
        }
        socket.close(None).unwrap();
    }
}

fn remove(path: &Path) -> io::Result<()> {
    let result = retry_fs(|| {
        if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
    });
    match result {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn retry_fs<T>(mut action: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let started = Instant::now();
    loop {
        match action() {
            Err(err)
                if err.kind() == io::ErrorKind::PermissionDenied
                    && started.elapsed() < Duration::from_secs(1) =>
            {
                // Windows can retain a deleted directory until watcher/read handles
                // close. Retry the individual operation, never the whole schedule.
                thread::sleep(Duration::from_millis(5));
            }
            result => return result,
        }
    }
}

fn operation(src: &Path, random: u64, revision: usize) -> io::Result<()> {
    let slot = (random >> 16) % 8;
    let script = src.join(format!("script{slot}.luau"));
    let folder = src.join(format!("folder{slot}"));
    let moved = src.join(format!("moved{slot}"));
    let swap = src.join(format!("swap{slot}.luau"));
    let model = src.join(format!("model{slot}.model.json"));
    let value = format!("return {revision}\n");
    match random % 13 {
        0 => retry_fs(|| fs::write(&script, &value)),
        1 => remove(&script),
        2 => {
            let temporary = src.join(format!("save{slot}.tmp"));
            fs::write(&temporary, value)?;
            retry_fs(|| fs::rename(&temporary, &script))
        }
        3 => {
            retry_fs(|| fs::create_dir_all(&folder))?;
            fs::write(folder.join("init.luau"), value)?;
            fs::write(folder.join("child.luau"), "return true\n")
        }
        4 => {
            remove(&moved)?;
            retry_fs(|| fs::create_dir_all(&folder))?;
            retry_fs(|| fs::rename(&folder, &moved))
        }
        5 => remove(&folder),
        6 => {
            remove(&swap)?;
            retry_fs(|| fs::write(&swap, &value))
        }
        7 => {
            remove(&swap)?;
            retry_fs(|| fs::create_dir_all(&swap))?;
            fs::write(swap.join("init.server.luau"), value)
        }
        8 => fs::write(
            model,
            r#"{"ClassName":"Folder","Children":[{"Name":"part","ClassName":"Part"}]}"#,
        ),
        9 => {
            let meta = src.join(format!("script{slot}.meta.json"));
            fs::write(meta, r#"{"properties":{"Archivable":false}}"#)
        }
        10 => {
            let nested = folder.join("deep").join("nested");
            retry_fs(|| fs::create_dir_all(&nested))?;
            fs::write(nested.join("init.client.luau"), value)?;
            remove(&folder)
        }
        11 => {
            remove(&moved)?;
            remove(&model)
        }
        _ => fs::write(
            src.join(format!("data{slot}.json")),
            format!(r#"{{"revision":{revision}}}"#),
        ),
    }
}

#[test]
fn seeded_filesystem_churn_converges() {
    let only_seed = std::env::var("ROJO_CHURN_SEED")
        .ok()
        .map(|seed| seed.parse::<u64>().unwrap());
    let operations = std::env::var("ROJO_CHURN_OPERATIONS")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(500);
    assert!(operations > 0);
    let run = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/churn")
        .join(run.to_string());
    for seed in only_seed.map_or_else(|| (1..=20).collect(), |seed| vec![seed]) {
        let artifacts = artifacts.join(format!("seed-{seed}"));
        let src = artifacts.join("project/src");
        fs::create_dir_all(&src).unwrap();
        let project = artifacts.join("project/default.project.json");
        fs::write(
            &project,
            r#"{"name":"Churn","tree":{"$className":"Folder","$path":"src"}}"#,
        )
        .unwrap();
        fs::write(src.join("canary.luau"), "return 0\n").unwrap();
        let mut transcript = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(artifacts.join("operations.log"))
            .unwrap();
        writeln!(transcript, "seed={seed} operations={operations}").unwrap();
        eprintln!("churn seed={seed}; artifacts={}", artifacts.display());
        let mut server = Server::start(&project, &artifacts);
        server.converge(&fresh_snapshot(&project), &artifacts);
        let mut random = seed;
        for revision in 1..=operations {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            writeln!(
                transcript,
                "revision={revision} operation={} slot={}",
                random % 13,
                (random >> 16) % 8
            )
            .unwrap();
            transcript.flush().unwrap();
            operation(&src, random, revision).unwrap();
            if revision % 100 == 0 || revision == operations {
                fs::write(src.join("canary.luau"), format!("return {revision}\n")).unwrap();
                fs::write(src.join("new_canary.luau"), format!("return {revision}\n")).unwrap();
                server.converge(&fresh_snapshot(&project), &artifacts);
                fs::remove_file(src.join("new_canary.luau")).unwrap();
                server.converge(&fresh_snapshot(&project), &artifacts);
            }
        }
    }
}
