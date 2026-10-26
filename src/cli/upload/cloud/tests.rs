use super::*;
use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

const KEY: &str = "LOCAL_TEST_KEY";

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
    delay: Duration,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: value.to_string().into_bytes(),
            headers: Vec::new(),
            delay: Duration::ZERO,
        }
    }
}
struct Request {
    headers: String,
    body: Vec<u8>,
}
struct Mock {
    base: String,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

fn read_request(stream: &mut TcpStream) -> Request {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "incomplete request headers");
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + length {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "incomplete request body");
        bytes.extend_from_slice(&buffer[..read]);
    }
    Request {
        headers,
        body: bytes[header_end..header_end + length].to_vec(),
    }
}

impl Mock {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut replies: VecDeque<_> = replies.into();
            let mut last =
                Reply::json(serde_json::json!({"path":"operations/update","done":false}));
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        captured.lock().unwrap().push(read_request(&mut stream));
                        if let Some(reply) = replies.pop_front() {
                            last = reply;
                        }
                        thread::sleep(last.delay);
                        let extra = last
                            .headers
                            .iter()
                            .map(|(key, value)| format!("{key}: {value}\r\n"))
                            .collect::<String>();
                        let head=format!("HTTP/1.1 {} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n",last.status,last.body.len());
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&last.body);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => panic!("local mock failed: {error}"),
                }
            }
        });
        Self {
            base,
            requests,
            stop,
            thread: Some(handle),
        }
    }
    fn cloud(&self) -> Cloud {
        let mut cloud = Cloud::new(KEY).unwrap();
        cloud.client = client_builder(KEY).unwrap().no_proxy().build().unwrap();
        cloud.base = self.base.clone();
        cloud.delay = Duration::from_millis(1);
        cloud.deadline = Duration::from_secs(2);
        cloud
    }
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

fn complete() -> Value {
    serde_json::json!({"path":"operations/update","done":true,"response":{"assetId":"42","path":"assets/42"}})
}
fn model() -> Target {
    Target::Model { asset_id: 42 }
}

#[test]
fn model_request_has_only_content_metadata_and_polls_operation_before_success() {
    let mock = Mock::new(vec![
        Reply::json(serde_json::json!({"path":"operations/update"})),
        Reply::json(complete()),
    ]);
    let binary = b"<roblox!\x00\xffbinary\r\nbody";
    mock.cloud().upload(binary.to_vec(), model()).unwrap();
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0]
        .headers
        .starts_with("PATCH /assets/v1/assets/42 HTTP/1.1"));
    assert!(requests[0]
        .headers
        .to_ascii_lowercase()
        .contains("x-api-key: local_test_key"));
    assert!(!requests[0].headers.to_ascii_lowercase().contains("cookie:"));
    let boundary = requests[0]
        .headers
        .lines()
        .find_map(|line| line.strip_prefix("content-type: multipart/form-data; boundary="))
        .unwrap();
    let (expected_boundary, _) = multipart(42, binary);
    assert_ne!(
        boundary, expected_boundary,
        "multipart boundary should be invocation-specific"
    );
    let body = &requests[0].body;
    let prefix=format!("--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\n\r\n{{\"assetId\":\"42\"}}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"fileContent\"; filename=\"model.rbxm\"\r\nContent-Type: model/x-rbxm\r\n\r\n");
    let mut expected = prefix.into_bytes();
    expected.extend_from_slice(binary);
    expected.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    assert_eq!(*body, expected);
    assert!(requests[1]
        .headers
        .starts_with("GET /assets/v1/operations/update HTTP/1.1"));
    assert!(requests[1].body.is_empty());
}

#[test]
fn place_request_sends_binary_mime_and_requires_positive_version_number() {
    for value in [
        serde_json::json!({"versionNumber":7}),
        serde_json::json!({}),
        serde_json::json!({"versionNumber":0}),
        serde_json::json!({"versionNumber":"7"}),
    ] {
        let valid = value["versionNumber"].as_u64() == Some(7);
        let mock = Mock::new(vec![Reply::json(value)]);
        assert_eq!(
            mock.cloud()
                .upload(
                    b"binary-place".to_vec(),
                    Target::Place {
                        universe_id: 10,
                        asset_id: 42
                    }
                )
                .is_ok(),
            valid
        );
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].headers.starts_with(
            "POST /universes/v1/10/places/42/versions?versionType=Published HTTP/1.1"
        ));
        assert!(requests[0]
            .headers
            .contains("content-type: application/octet-stream"));
        assert_eq!(requests[0].body, b"binary-place");
    }
}

#[test]
fn asset_operation_errors_and_malformed_envelopes_never_succeed() {
    for value in [
        serde_json::json!({"path":"operations/update","done":true,"error":{"code":3,"message":"invalid model"}}),
        serde_json::json!({"path":"operations/update","done":true}),
        serde_json::json!({"path":"operations/update","done":true,"response":{"assetId":"43"}}),
        serde_json::json!({"path":"operations/update","done":true,"response":{"assetId":"42","path":"assets/43"}}),
        serde_json::json!({"path":"operations/update","done":true,"response":{"assetId":false,"path":"assets/42"}}),
        serde_json::json!({"path":"operations/update","done":"true"}),
        serde_json::json!({"path":"https://elsewhere.invalid/steal"}),
        serde_json::json!({"path":"operations/../steal"}),
        serde_json::json!({"path":"operations/update?key=bad"}),
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        let mock = Mock::new(vec![Reply::json(value.clone())]);
        assert!(mock.cloud().upload(vec![], model()).is_err(), "{value}");
        assert_eq!(
            mock.requests.lock().unwrap().len(),
            1,
            "invalid operation triggered polling"
        );
    }
}

#[test]
fn operation_polls_reject_changed_paths_and_malformed_poll_response() {
    for reply in [
        serde_json::json!({"path":"operations/other","done":true,"response":{"assetId":"42"}}),
        serde_json::json!([]),
    ] {
        let mock = Mock::new(vec![
            Reply::json(serde_json::json!({"path":"operations/update"})),
            Reply::json(reply),
        ]);
        assert!(mock.cloud().upload(vec![], model()).is_err());
        assert_eq!(mock.requests.lock().unwrap().len(), 2);
    }
}

#[test]
fn polling_timeout_reports_operation_without_repeating_the_mutation() {
    let mock = Mock::new(vec![Reply::json(
        serde_json::json!({"path":"operations/update","done":false}),
    )]);
    let mut cloud = mock.cloud();
    cloud.deadline = Duration::from_millis(250);
    let started = Instant::now();
    let error = cloud.upload(vec![], model()).unwrap_err().to_string();
    assert!(error.contains("operations/update"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(1));
    let requests = mock.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.headers.starts_with("PATCH "))
            .count(),
        1
    );
}

#[test]
fn http_failures_are_bounded_redacted_and_never_retried_as_mutations() {
    for status in [401, 403, 404, 429, 500] {
        let mut reply = Reply::json(serde_json::json!({"message":format!("reflected {KEY}")}));
        reply.status = status;
        let mock = Mock::new(vec![reply]);
        let error = mock
            .cloud()
            .upload(vec![], model())
            .unwrap_err()
            .to_string();
        assert!(error.contains(&status.to_string()), "{error}");
        assert!(!error.contains(KEY), "{error}");
        assert!(error.contains("[REDACTED]"), "{error}");
        assert_eq!(mock.requests.lock().unwrap().len(), 1);
    }
    let mut reply = Reply::json(Value::Null);
    reply.status = 500;
    reply.body = vec![b'x'; 20000];
    let mock = Mock::new(vec![reply]);
    assert!(
        mock.cloud()
            .upload(vec![], model())
            .unwrap_err()
            .to_string()
            .len()
            < 9000
    );
}

#[test]
fn redirects_do_not_forward_the_api_key() {
    let trap = Mock::new(vec![Reply::json(complete())]);
    let mut reply = Reply::json(Value::Null);
    reply.status = 307;
    reply
        .headers
        .push(("Location".into(), format!("{}/steal", trap.base)));
    let mock = Mock::new(vec![reply]);
    let error = mock
        .cloud()
        .upload(vec![], model())
        .unwrap_err()
        .to_string();
    assert!(error.contains("307"), "{error}");
    assert!(trap.requests.lock().unwrap().is_empty());
}

#[test]
fn oversized_and_invalid_json_responses_fail_before_success() {
    for body in [
        vec![b'x'; RESPONSE_LIMIT as usize + 1],
        b"not json".to_vec(),
    ] {
        let mut reply = Reply::json(Value::Null);
        reply.body = body;
        let mock = Mock::new(vec![reply]);
        assert!(mock.cloud().upload(vec![], model()).is_err());
    }
}

#[test]
fn api_key_validation_and_debug_redaction_do_not_expose_the_key() {
    for key in ["", "  ", "bad\r\nkey"] {
        assert!(Cloud::new(key).is_err());
    }
    let cloud = Cloud::new(KEY).unwrap();
    assert!(!format!("{:?}", cloud.client).contains(KEY));
    let mut cloud = cloud;
    cloud.key = "quote\"key".into();
    let error = cloud.redact(r#"raw quote"key JSON quote\"key"#);
    assert!(!error.contains("quote"), "{error}");
}

#[test]
fn slow_poll_request_is_bounded_by_remaining_operation_deadline() {
    let mut slow = Reply::json(complete());
    slow.delay = Duration::from_millis(250);
    let mock = Mock::new(vec![
        Reply::json(serde_json::json!({"path":"operations/update"})),
        slow,
    ]);
    let mut cloud = mock.cloud();
    cloud.deadline = Duration::from_millis(100);
    let started = Instant::now();
    let error = cloud.upload(vec![], model()).unwrap_err().to_string();
    assert!(error.contains("operations/update"), "{error}");
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(mock.requests.lock().unwrap().len(), 2);
}
