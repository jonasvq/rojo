use std::{fs, path::Path, process::Command};

fn build(project: &Path, output: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("build")
        .arg(project)
        .arg("--output")
        .arg(output)
        .output()
        .unwrap()
}

#[test]
fn cli_build_accepts_project_stems_with_json_jsonc_and_unicode_names() {
    for (name, suffix) in [
        ("custom", ".project.json"),
        ("custom.client", ".project.jsonc"),
        ("Überblick", ".project.json"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(format!("{name}{suffix}"));
        fs::write(
            file,
            r#"{"name":"StemProject","tree":{"$className":"Folder"}}"#,
        )
        .unwrap();
        let output = dir.path().join("out.rbxmx");
        let result = build(&dir.path().join(name), &output);
        assert!(
            result.status.success(),
            "{name}{suffix}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(fs::read_to_string(output).unwrap().contains("StemProject"));
    }
}

#[test]
fn project_stem_prefers_existing_paths_then_json_before_jsonc() {
    let dir = tempfile::tempdir().unwrap();
    let stem = dir.path().join("project");
    fs::create_dir(&stem).unwrap();
    fs::write(
        stem.join("default.project.json"),
        r#"{"name":"DirectoryWins","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("project.project.json"),
        r#"{"name":"JsonWins","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("project.project.jsonc"),
        r#"{"name":"Jsonc","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    let output = dir.path().join("out.rbxmx");
    assert!(build(&stem, &output).status.success());
    assert!(fs::read_to_string(&output)
        .unwrap()
        .contains("DirectoryWins"));
    fs::remove_file(stem.join("default.project.json")).unwrap();
    fs::remove_dir(&stem).unwrap();
    assert!(build(&stem, &output).status.success());
    assert!(fs::read_to_string(&output).unwrap().contains("JsonWins"));
    fs::write(&stem, b"existing non-project file").unwrap();
    assert!(
        !build(&stem, &output).status.success(),
        "an existing file must not be silently reinterpreted"
    );
    fs::remove_file(&stem).unwrap();
    fs::write(dir.path().join("project.project.json"), b"invalid json").unwrap();
    assert!(
        !build(&stem, &output).status.success(),
        "malformed preferred JSON must not fall through to JSONC"
    );
}

#[test]
fn other_project_reading_commands_resolve_stems_without_guessing_input_output_names() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("custom.project.jsonc");
    fs::write(
        &project,
        r#"{"name":"StemProject","tree":{"$className":"Folder","Child":{"$className":"ModuleScript","$properties":{"Source":"return 1"}}}}"#,
    )
    .unwrap();
    let stem = dir.path().join("custom");
    let model = dir.path().join("input.rbxm");
    assert!(build(&project, &model).status.success());
    let map = dir.path().join("sourcemap.json");
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("sourcemap")
        .arg(&stem)
        .arg("--output")
        .arg(&map)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(map).unwrap()).unwrap();
    assert_eq!(json["name"], "StemProject");
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("syncback")
        .arg(&stem)
        .arg("--input")
        .arg(&model)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("fmt-project")
        .arg(&stem)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read_to_string(&project).unwrap().contains('\n'));
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("upload")
        .arg(&stem)
        .args(["--asset_id", "42"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--api_key"));
    let unknown_output = dir.path().join("output");
    fs::write(
        dir.path().join("output.project.json"),
        b"preserve output names",
    )
    .unwrap();
    assert!(!build(&stem, &unknown_output).status.success());
    assert_eq!(
        fs::read(dir.path().join("output.project.json")).unwrap(),
        b"preserve output names"
    );
}

#[test]
fn existing_place_overlay_also_accepts_project_stem() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("patch.project.json");
    fs::write(&project,r#"{"name":"Place","tree":{"$className":"DataModel","Workspace":{"$className":"Workspace"}}}"#).unwrap();
    let input = dir.path().join("input.rbxlx");
    let output = dir.path().join("output.rbxlx");
    assert!(build(&project, &input).status.success());
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("build")
        .arg(dir.path().join("patch"))
        .arg("--input")
        .arg(input)
        .arg("--output")
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn live_server_loads_the_resolved_project_before_binding() {
    use std::{
        net::TcpListener,
        process::Child,
        time::{Duration, Instant},
    };
    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("custom.project.json"),
        r#"{"name":"StemLive","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log = fs::File::create(dir.path().join("serve.log")).unwrap();
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_rojo"))
            .arg("serve")
            .arg(dir.path().join("custom"))
            .arg("--port")
            .arg(port.to_string())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(250))
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/api/rojo"))
            .send()
        {
            if response.status().is_success() {
                let info: serde_json::Value =
                    rmp_serde::from_slice(&response.bytes().unwrap()).unwrap();
                assert_eq!(info["projectName"], "StemLive");
                break;
            }
        }
        assert!(process.0.try_wait().unwrap().is_none(), "stem serve exited");
        assert!(Instant::now() < deadline, "stem serve did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
}
