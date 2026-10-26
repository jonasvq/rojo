use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use reqwest::{blocking::Client, StatusCode};
use serde_json::{json, Value};

struct Server {
    child: std::process::Child,
    dir: tempfile::TempDir,
    client: Client,
    base: String,
    session: String,
    id: String,
    origin: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let path = self.path();
        if let Ok(metadata) = fs::metadata(&path) {
            let mut permissions = metadata.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

impl Server {
    fn new() -> Self {
        Self::with_body_limit(None)
    }

    fn with_body_limit(limit: Option<usize>) -> Self {
        Self::with_fixture(
            limit,
            json!({"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Example.luau"}}}),
            &[],
        )
    }

    fn with_fixture(limit: Option<usize>, project: Value, files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Example.luau"), "return 1").unwrap();
        for (path, contents) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        fs::write(
            dir.path().join("default.project.json"),
            serde_json::to_vec(&project).unwrap(),
        )
        .unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut command = Command::new(env!("CARGO_BIN_EXE_rojo"));
        command
            .current_dir(dir.path())
            .args(["serve", "--port", &port.to_string()]);
        if let Some(limit) = limit {
            command.args(["--max-request-body-bytes", &limit.to_string()]);
        }
        let child = command
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.path().join("server.log")).unwrap())
            .spawn()
            .unwrap();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let mut server = Self {
            child,
            dir,
            client,
            base: format!("http://127.0.0.1:{port}"),
            session: String::new(),
            id: String::new(),
            origin: uuid::Uuid::new_v4().to_string(),
        };
        let deadline = Instant::now() + Duration::from_secs(12);
        let info: Value = loop {
            if let Ok(response) = server
                .client
                .get(format!("{}/api/rojo", server.base))
                .send()
            {
                if response.status().is_success() {
                    break rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
                }
            }
            assert!(Instant::now() < deadline && server.child.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(20));
        };
        server.session = info["sessionId"].as_str().unwrap().into();
        let tree = server.get(&format!(
            "/api/read/{}",
            info["rootInstanceId"].as_str().unwrap()
        ));
        server.id = tree["instances"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, node)| node["Name"] == "Example")
            .unwrap()
            .0
            .clone();
        server
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("Example.luau")
    }

    fn get(&self, route: &str) -> Value {
        let response = self
            .client
            .get(format!("{}{route}", self.base))
            .send()
            .unwrap();
        assert!(
            response.status().is_success(),
            "GET {route}: {}",
            response.status()
        );
        rmp_serde::from_slice(&response.bytes().unwrap()).unwrap()
    }

    fn source(&self) -> Value {
        self.get(&format!("/api/write-receipts/source/{}", self.id))
    }

    fn request(&self, base: &Value, source: &str) -> Value {
        json!({"sessionId":self.session,"originId":self.origin,"requestId":uuid::Uuid::new_v4().to_string(),
            "instanceId":self.id,"className":base["className"],"sourceHash":base["sourceHash"],"bindingHash":base["bindingHash"],"source":source})
    }

    fn submit(&self, request: &Value) -> Value {
        let response = self
            .client
            .post(format!("{}/api/write-receipts", self.base))
            .body(rmp_serde::to_vec_named(request).unwrap())
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        rmp_serde::from_slice(&response.bytes().unwrap()).unwrap()
    }

    fn terminal(&self, receipt: &Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let view = self.get(&format!(
                "/api/write-receipts/{}?sessionId={}&originId={}",
                receipt["receiptId"].as_str().unwrap(),
                self.session,
                self.origin
            ));
            if view["state"] != "pending" {
                return view;
            }
            assert!(Instant::now() < deadline, "receipt stayed pending: {view}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn withdrawn_managed_write_routes_are_absent_while_source_receipts_remain_available() {
    let server = Server::new();
    for route in [
        format!("binding/{}?sessionId={}", server.id, server.session),
        "plan".into(),
        "commit".into(),
    ] {
        for method in [reqwest::Method::GET, reqwest::Method::POST] {
            let response = server
                .client
                .request(
                    method,
                    format!("{}/api/managed-writes/{route}", server.base),
                )
                .body("{}")
                .send()
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{route}");
        }
    }
    let info = server.get("/api/rojo");
    assert!(!info["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|capability| capability == "managedWritesV1"));
    let source = server.source();
    assert_eq!(
        server.terminal(&server.submit(&server.request(&source, "return 2")))["state"],
        "applied"
    );
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 2");
}

#[test]
fn denied_write_reports_terminal_io_failure_instead_of_queued_success() {
    let server = Server::new();
    let base = server.source();
    let mut permissions = fs::metadata(server.path()).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(server.path(), permissions).unwrap();
    let result = server.terminal(&server.submit(&server.request(&base, "return 2")));
    assert_eq!(result["state"], "ioError", "{result}");
    assert!(result["reason"].as_str().unwrap().contains("write"));
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 1");
    assert_eq!(server.source()["source"], "return 1");
}

#[test]
fn explicit_authority_revokes_source_receipts_and_restores_legacy_only_when_removed() {
    let server = Server::new();
    let source_base = server.source();
    let original =
        json!({"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Example.luau"}}});
    let mut project = original.clone();
    project["syncAuthority"] =
        json!([{"className":"LuaSourceContainer","properties":"filesystem"}]);
    fs::write(
        server.dir.path().join("default.project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let denied =
        server.terminal(&server.submit(&server.request(&source_base, "return 'forbidden'")));
    assert_eq!(denied["state"], "rejected");
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 1");
    // Unsupported live policy changes cannot fall back to the old grant.
    project["syncAuthority"][0]["properties"] = "studio".into();
    fs::write(
        server.dir.path().join("default.project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let rejected =
        server.terminal(&server.submit(&server.request(&source_base, "return 'still forbidden'")));
    assert_eq!(rejected["state"], "rejected");
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 1");
    fs::write(
        server.dir.path().join("default.project.json"),
        serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    let current = server.source();
    assert_eq!(
        server.terminal(&server.submit(&server.request(&current, "return 2")))["state"],
        "applied"
    );
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 2");
}

#[test]
fn stale_external_base_and_two_writers_conflict_without_overwriting() {
    let server = Server::new();
    let base = server.source();
    let first_request = server.request(&base, "return 2");
    let receipt = server.submit(&first_request);
    assert_eq!(server.terminal(&receipt)["state"], "applied");
    assert_eq!(
        server.terminal(&server.submit(&server.request(&base, "return 3")))["state"],
        "conflict"
    );
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 2");
    let current = server.source();
    fs::write(server.path(), "return 4").unwrap();
    assert_eq!(
        server.terminal(&server.submit(&server.request(&current, "return 5")))["state"],
        "conflict"
    );
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 4");
    // Retries identify the original persisted write, even after a later editor change.
    let retry = server.submit(&first_request);
    assert_eq!(retry["receiptId"], receipt["receiptId"]);
    assert_eq!(server.terminal(&retry)["state"], "applied");
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 4");
}

#[test]
fn malformed_sessions_trailing_bodies_and_reused_ids_never_write() {
    let server = Server::new();
    let base = server.source();
    let request = server.request(&base, "return 2");
    let receipt = server.submit(&request);
    assert_eq!(server.terminal(&receipt)["state"], "applied");
    let mut different = request.clone();
    different["source"] = "return 3".into();
    let response = server
        .client
        .post(format!("{}/api/write-receipts", server.base))
        .body(rmp_serde::to_vec_named(&different).unwrap())
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let mut wrong_session = request.clone();
    wrong_session["sessionId"] = uuid::Uuid::new_v4().to_string().into();
    let mut extra_field = request.clone();
    extra_field["unexpected"] = true.into();
    for invalid in [wrong_session, extra_field] {
        let response = server
            .client
            .post(format!("{}/api/write-receipts", server.base))
            .body(rmp_serde::to_vec_named(&invalid).unwrap())
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let mut bytes = rmp_serde::to_vec_named(&request).unwrap();
    bytes.push(0xc0);
    assert_eq!(
        server
            .client
            .post(format!("{}/api/write-receipts", server.base))
            .body(bytes)
            .send()
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let foreign = format!(
        "{}/api/write-receipts/{}?sessionId={}&originId={}",
        server.base,
        receipt["receiptId"].as_str().unwrap(),
        server.session,
        uuid::Uuid::new_v4()
    );
    assert_eq!(
        server.client.get(foreign).send().unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 2");
}

#[test]
fn changed_binding_aliases_and_source_overrides_are_rejected_before_writing() {
    let server = Server::new();
    let base = server.source();
    fs::write(server.dir.path().join("Other.luau"), "return 1").unwrap();
    let project = server.dir.path().join("default.project.json");
    fs::write(
        &project,
        br#"{"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Other.luau"}}}"#,
    )
    .unwrap();
    let result = server.terminal(&server.submit(&server.request(&base, "return 3")));
    assert_eq!(result["state"], "conflict", "{result}");
    assert_eq!(
        fs::read_to_string(server.dir.path().join("Other.luau")).unwrap(),
        "return 1"
    );
    fs::write(&project, br#"{"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Example.luau"},"Alias":{"$path":"./Example.luau"}}}"#).unwrap();
    assert_eq!(
        server.terminal(&server.submit(&server.request(&base, "return 3")))["state"],
        "rejected"
    );
    fs::write(&project, br#"{"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Example.luau","$properties":{"Source":"return 1"}}}}"#).unwrap();
    assert_eq!(
        server.terminal(&server.submit(&server.request(&base, "return 3")))["state"],
        "rejected"
    );
    fs::write(
        &project,
        br#"{"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Example.luau"}}}"#,
    )
    .unwrap();
    fs::write(
        server.dir.path().join("Example.meta.jsonc"),
        br#"{"properties":{"Source":"return 1"}}"#,
    )
    .unwrap();
    assert_eq!(
        server.terminal(&server.submit(&server.request(&base, "return 3")))["state"],
        "rejected"
    );
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 1");
}

#[test]
fn native_init_and_canonical_text_module_writes_read_back_their_representation() {
    let server = Server::new();
    let project = server.dir.path().join("default.project.json");
    fs::create_dir(server.dir.path().join("Directory")).unwrap();
    let init = server.dir.path().join("Directory/init.luau");
    fs::write(&init, "return 1\r\n").unwrap();
    fs::write(
        &project,
        br#"{"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Directory"}}}"#,
    )
    .unwrap();
    let base = server.source();
    assert_eq!(base["source"], "return 1\n");
    let result = server.terminal(&server.submit(&server.request(&base, "return 2\r\n")));
    assert_eq!(result["state"], "applied", "{result}");
    assert_eq!(server.source()["source"], "return 2\n");
    assert_eq!(fs::read_to_string(&init).unwrap(), "return 2\r\n");
    fs::write(server.dir.path().join("text.txt"), "hello\r\n").unwrap();
    fs::write(&project, br#"{"name":"Root","syncRules":[{"pattern":"**/*.txt","use":"textModule","suffix":".txt"}],"tree":{"$className":"Folder","Example":{"$path":"text.txt"}}}"#).unwrap();
    let base = server.source();
    let bad = server.terminal(&server.submit(&server.request(&base, "return game")));
    assert_eq!(bad["state"], "rejected", "{bad}");
    assert_eq!(bad["mayHaveChanged"], false);
    assert_eq!(
        fs::read_to_string(server.dir.path().join("text.txt")).unwrap(),
        "hello\r\n"
    );
    let good =
        server.terminal(&server.submit(&server.request(&base, "return \"world\\013\\010\"\n")));
    assert_eq!(good["state"], "applied", "{good}");
    assert_eq!(
        fs::read_to_string(server.dir.path().join("text.txt")).unwrap(),
        "world\r\n"
    );
    let source = server.source();
    let unchanged =
        server.terminal(&server.submit(&server.request(&base, source["source"].as_str().unwrap())));
    assert_eq!(unchanged["state"], "unchanged");
}

#[test]
fn negotiated_read_bindings_share_the_accepted_source_and_preserve_legacy_shapes() {
    let server = Server::new();
    let info = server.get("/api/rojo");
    let root = info["rootInstanceId"].as_str().unwrap();
    assert!(info["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("writeReceiptsV1")));
    let legacy = server.get(&format!("/api/read/{root}"));
    assert!(legacy.get("writeSources").is_none());
    let extended = server.get(&format!(
        "/api/read/{root}?writeReceiptsV1=true&showPaths=true"
    ));
    let view = server.source();
    assert_eq!(
        extended["writeSources"][&server.id]["sourceHash"],
        view["sourceHash"]
    );
    assert_eq!(
        extended["writeSources"][&server.id]["bindingHash"],
        view["bindingHash"]
    );
    assert!(extended["instances"][&server.id]["FilePaths"]
        .as_array()
        .is_some());
    assert_eq!(
        extended["instances"][&server.id]["Properties"]["Source"]["String"],
        view["source"]
    );
    // The public legacy DTO still accepts the negotiated response.
    let bytes = server
        .client
        .get(format!(
            "{}/api/read/{root}?writeReceiptsV1=true",
            server.base
        ))
        .send()
        .unwrap()
        .bytes()
        .unwrap();
    let mut decoder = rmp_serde::Deserializer::new(bytes.as_ref()).with_human_readable();
    let _: librojo::web_api::ReadResponse<'_> =
        serde::Deserialize::deserialize(&mut decoder).unwrap();
}

#[test]
fn receipt_route_enforces_the_configured_body_limit_and_stays_healthy() {
    let server = Server::with_body_limit(Some(512));
    let base = server.source();
    let request = server.request(&base, &"x".repeat(2048));
    let response = server
        .client
        .post(format!("{}/api/write-receipts", server.base))
        .body(rmp_serde::to_vec_named(&request).unwrap())
        .send()
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 1");
    let accepted = server.terminal(&server.submit(&server.request(&base, "return 2")));
    assert_eq!(accepted["state"], "applied");
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 2");
}

#[test]
fn rule_and_mask_reloads_change_receipt_authority_without_changing_source() {
    let server = Server::new();
    let original = server.source();
    let wait_changed = |previous: &Value| {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let source = server.source();
            if source["bindingHash"] != previous["bindingHash"] {
                assert_eq!(source["sourceHash"], original["sourceHash"]);
                return source;
            }
            assert!(Instant::now() < deadline, "authority did not refresh");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let path = server.dir.path().join("default.project.json");
    fs::write(&path, br#"{"name":"Root","syncRules":[{"pattern":"*.luau","defaults":{"tags":["Shared"],"attributes":{"Owner":"shared"}}}],"tree":{"$className":"Folder","Example":{"$path":"Example.luau"}}}"#).unwrap();
    let rules = wait_changed(&original);
    let meta = server.dir.path().join("Example.meta.jsonc");
    fs::write(
        &meta,
        br#"{"ruleDefaults":{"disabledProperties":["Tags"],"disabledAttributes":["Owner"]}}"#,
    )
    .unwrap();
    let masked = wait_changed(&rules);
    let result = server.terminal(&server.submit(&server.request(&rules, "return 'stale'")));
    assert_eq!(result["state"], "conflict", "{result}");
    fs::remove_file(meta).unwrap();
    let restored = wait_changed(&masked);
    assert_eq!(restored["bindingHash"], rules["bindingHash"]);
    fs::write(
        &path,
        br#"{"name":"Root","tree":{"$className":"Folder","Example":{"$path":"Example.luau"}}}"#,
    )
    .unwrap();
    let removed = wait_changed(&restored);
    assert_eq!(removed["bindingHash"], original["bindingHash"]);
    assert_eq!(fs::read_to_string(server.path()).unwrap(), "return 1");
}
