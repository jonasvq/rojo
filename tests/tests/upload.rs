use std::{
    fs::{self, File},
    io::{self, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::rojo_test::io_util::ROJO_PATH;

fn upload_through_local_proxy(class_name: &str, api_key: bool) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("default.project.json");
    fs::write(
        &project,
        format!(r#"{{"name":"Upload","tree":{{"$className":"{class_name}"}}}}"#),
    )
    .unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let log_path = dir.path().join("upload.log");
    let log = File::create(&log_path).unwrap();
    let mut command = Command::new(ROJO_PATH);

    command.arg("upload");
    if api_key {
        command.args(["--api_key", "LOCAL_TEST_KEY"]);
    }
    let mut child = command
        .arg(&project)
        .args(["--cookie", "FAKE_TEST_COOKIE", "--asset_id", "1"])
        // Keep the test local and avoid reading any installed authentication.
        .env("HTTPS_PROXY", &proxy_url)
        .env("HTTP_PROXY", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env("NO_PROXY", "")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    let mut contacted = false;
    let started = Instant::now();
    let status = loop {
        match proxy.accept() {
            Ok((mut stream, _)) => {
                contacted = true;
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                // Refuse the CONNECT tunnel: no request reaches Roblox.
                let _ = stream.write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => panic!("proxy accept failed: {err}"),
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() >= Duration::from_secs(10) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("upload did not terminate through the local proxy");
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success());
    (contacted, fs::read_to_string(log_path).unwrap())
}

#[test]
fn legacy_place_upload_is_rejected_before_any_network_request() {
    let (contacted, log) = upload_through_local_proxy("DataModel", false);
    assert!(
        log.contains("Place uploads require Open Cloud")
            && log.contains("--api_key")
            && log.contains("--universe_id"),
        "{log}"
    );
    assert!(!contacted, "unsupported place upload contacted the network");
}

#[test]
fn legacy_model_upload_is_rejected_before_any_network_request() {
    let (contacted, log) = upload_through_local_proxy("Folder", false);
    assert!(
        !contacted,
        "model cookie fallback contacted the network: {log}"
    );
    assert!(log.contains("--api_key"), "{log}");
    assert!(!log.contains("FAKE_TEST_COOKIE"), "secret leaked: {log}");
}

#[test]
fn model_api_key_upload_uses_open_cloud_without_a_universe_id() {
    let (contacted, log) = upload_through_local_proxy("Folder", true);
    assert!(
        contacted,
        "model key was rejected before the local proxy: {log}"
    );
    assert!(!log.contains("--universe_id must be provided"), "{log}");
    assert!(!log.contains("LOCAL_TEST_KEY"), "secret leaked: {log}");
}
