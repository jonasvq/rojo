use clap::Parser;
use librojo::cli::Options;
use serde_json::{json, Value};
use std::{
    fs,
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    dir: tempfile::TempDir,
    client: reqwest::blocking::Client,
    url: String,
    info: Value,
}

impl Server {
    fn new(flags: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/runtime")).unwrap();
        fs::write(
            dir.path().join("src/runtime/init.client.luau"),
            "print('source')",
        )
        .unwrap();
        fs::write(dir.path().join("default.project.json"), r#"{"name":"Output","tree":{"$className":"DataModel","ReplicatedFirst":{"$className":"ReplicatedFirst","$path":"src"}}}"#).unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .arg("serve")
            .arg(dir.path())
            .args(["--address", "127.0.0.1", "--port", &port.to_string()])
            .args(flags)
            .env("RUST_LOG", "error")
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.path().join("stderr.log")).unwrap())
            .spawn()
            .unwrap();
        let mut server = Self {
            child,
            dir,
            client: reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            url: format!("http://127.0.0.1:{port}"),
            info: Value::Null,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(response) = server.client.get(format!("{}/api/rojo", server.url)).send() {
                if response.status().is_success() {
                    server.info = rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
                    return server;
                }
            }
            assert!(
                Instant::now() < deadline,
                "server startup: {}",
                server.logs()
            );
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "server died: {}",
                server.logs()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn send(&self, body: &Value) -> reqwest::blocking::Response {
        self.client
            .post(format!("{}/api/output", self.url))
            .body(rmp_serde::to_vec_named(body).unwrap())
            .send()
            .unwrap()
    }
    fn batch(&self, entries: Value) -> Value {
        json!({"sessionId": self.info["sessionId"], "entries": entries})
    }
    fn logs(&self) -> String {
        fs::read_to_string(self.dir.path().join("stderr.log")).unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn output_negotiation_is_opt_in_and_flags_select_exact_levels() {
    let default = Server::new(&[]);
    assert_eq!(default.info["protocolVersion"], 5);
    assert!(default.info.get("outputLogs").is_none());
    assert!(!default.info["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("studioOutputV1")));
    assert_eq!(default.send(&json!({})).status(), 404);
    for (flag, levels) in [
        ("--sync-error-log", json!(["Error"])),
        ("--sync-warn-log", json!(["Warning"])),
        ("--sync-info-log", json!(["Info"])),
        ("--sync-output-log", json!(["Output"])),
        ("--sync-logs", json!(["Error", "Warning", "Info", "Output"])),
    ] {
        let server = Server::new(&[flag]);
        assert_eq!(server.info["outputLogs"], levels, "{flag}");
        assert!(server.info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("studioOutputV1")));
    }
}

#[test]
fn output_validates_whole_batches_limits_and_session_before_printing() {
    let server = Server::new(&["--sync-error-log"]);
    let valid = json!({"level":"Error", "message":"DO_NOT_PARTIALLY_PRINT"});
    let mut malformed = vec![
        json!([]),
        json!(vec![valid.clone(); 65]),
        json!([valid.clone(), {"level":"Error","message":"x".repeat(8193)}]),
        json!([valid.clone(), {"level":"Other","message":"invalid"}]),
        json!([valid.clone(), {"level":"Error","message":"invalid","extra":true}]),
    ];
    malformed.push(json!([{"level":"Error","message":42}]));
    for entries in malformed {
        let response = server.send(&server.batch(entries));
        assert_eq!(response.status(), 400);
        let error: Value = rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
        assert!(error.is_object());
    }
    let mut wrong = server.batch(json!([valid.clone()]));
    wrong["sessionId"] = json!("stale");
    assert_eq!(server.send(&wrong).status(), 400);
    wrong = server.batch(json!([valid]));
    wrong["unknown"] = json!(true);
    assert_eq!(server.send(&wrong).status(), 400);
    let response = server
        .client
        .post(format!("{}/api/output", server.url))
        .body(vec![0; 1024 * 1024 + 1])
        .send()
        .unwrap();
    assert_eq!(response.status(), 413);
    assert!(!server.logs().contains("DO_NOT_PARTIALLY_PRINT"));
    let response =
        server.send(&server.batch(json!([{"level":"Output","message":"DISABLED_OUTPUT"}])));
    assert_eq!(response.status(), 200);
    assert!(!server.logs().contains("DISABLED_OUTPUT"));
    let small = Server::new(&["--sync-logs", "--max-request-body-bytes", "128"]);
    assert_eq!(
        small
            .send(&small.batch(json!([{"level":"Error","message":"x".repeat(200)}])))
            .status(),
        413
    );
}

#[test]
fn output_maps_live_sources_and_escapes_terminal_controls_independent_of_rust_log() {
    let server = Server::new(&["--sync-logs"]);
    let response = server.send(&server.batch(json!([
        {"level":"Error","message":"ReplicatedFirst.runtime:27: failed\n  Script 'ReplicatedFirst.runtime', Line 28 - function run"},
        {"level":"Output","message":"CONTROL\u{1b}[31m\r\u{8}\tOK"}
    ])));
    assert_eq!(response.status(), 200);
    let ack: Value = rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
    assert_eq!(ack["sessionId"], server.info["sessionId"]);
    let logs = server.logs().replace('\\', "/");
    let source = server
        .dir
        .path()
        .join("src/runtime/init.client.luau")
        .to_string_lossy()
        .replace('\\', "/");
    assert!(
        logs.contains(&format!("[Studio Error] {source}:27: failed")),
        "{logs}"
    );
    assert!(
        logs.contains(&format!("[Studio Error]   {source}:28 - function run")),
        "{logs}"
    );
    assert!(logs.contains("[Studio Output] CONTROL"));
    assert!(!logs.contains('\u{1b}') && !logs.contains('\u{8}'));
    fs::rename(
        server.dir.path().join("src/runtime"),
        server.dir.path().join("src/renamed"),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = server
            .client
            .get(format!(
                "{}/api/read/{}",
                server.url,
                server.info["rootInstanceId"].as_str().unwrap()
            ))
            .send()
            .unwrap();
        let body: Value = rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
        if body["instances"]
            .as_object()
            .unwrap()
            .values()
            .any(|node| node["Name"] == "renamed")
        {
            break;
        }
        assert!(Instant::now() < deadline, "rename not observed: {body}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        server
            .send(&server.batch(json!([
                {"level":"Error","message":"ReplicatedFirst.renamed:9: RENAMED"},
                {"level":"Error","message":"ReplicatedFirst.runtime:7: OLD_NAME_FALLBACK"}
            ])))
            .status(),
        200
    );
    let logs = server.logs().replace('\\', "/");
    assert!(
        logs.contains("src/renamed/init.client.luau:9: RENAMED"),
        "{logs}"
    );
    assert!(logs.contains("ReplicatedFirst.runtime:7: OLD_NAME_FALLBACK"));
}

#[test]
fn serve_accepts_all_studio_output_flags() {
    for flag in [
        "--sync-logs",
        "--sync-error-log",
        "--sync-warn-log",
        "--sync-info-log",
        "--sync-output-log",
    ] {
        assert!(
            Options::try_parse_from(["rojo", "serve", flag]).is_ok(),
            "{flag}"
        );
    }
}
