use std::process::Command;

#[test]
fn cli_exposes_the_open_workflow() {
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .args(["open", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("--project") && help.contains("--port"));
}

#[cfg(windows)]
mod windows {
    use rbx_dom_weak::{types::Variant, InstanceBuilder, WeakDom};
    use std::{
        fs,
        net::TcpListener,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::OnceLock,
        time::{Duration, Instant},
    };

    fn studio() -> &'static Path {
        static INSTALL: OnceLock<tempfile::TempDir> = OnceLock::new();
        INSTALL
            .get_or_init(|| {
                let dir = tempfile::tempdir().unwrap();
                fs::create_dir(dir.path().join("content")).unwrap();
                let source = dir.path().join("fake_studio.rs");
                fs::write(&source, include_str!("fixtures/open_fake_studio.rs")).unwrap();
                let output = Command::new("rustc")
                    .arg("--edition=2021")
                    .arg(&source)
                    .arg("-o")
                    .arg(dir.path().join("RobloxStudioBeta.exe"))
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                dir
            })
            .path()
    }

    fn port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn fixture(dir: &Path, port: u16) {
        fs::write(dir.join("default.project.json"), format!(r#"{{
            "name":"OpenFixture","servePort":{port},"tree":{{"$className":"DataModel",
                "ReplicatedStorage":{{"$className":"ReplicatedStorage",
                    "Target":{{"$className":"Folder","$attributes":{{"Rojo_Id":"target"}}}},
                    "Pointer":{{"$className":"ObjectValue","$attributes":{{"Rojo_Target_Value":"target"}}}}
                }}
            }}
        }}"#)).unwrap();
    }

    fn command(dir: &Path, install: &Path, port: u16) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rojo"));
        command
            .current_dir(dir)
            .arg("open")
            .arg("--skip-plugin-install")
            .env("ROBLOX_STUDIO_PATH", install)
            .env("ROJO_OPEN_TEST_PORT", port.to_string())
            .env("ROJO_OPEN_TEST_LOG", dir.join("launch.log"));
        command
    }

    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn start(dir: &Path, port: u16, args: &[&str]) -> (Process, PathBuf) {
        let mut process = Process(
            command(dir, studio(), port)
                .args(args)
                .stdout(Stdio::null())
                .stderr(fs::File::create(dir.join("stderr.log")).unwrap())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Ok(log) = fs::read_to_string(dir.join("launch.log")) {
                if log.contains("HTTP_200") {
                    return (process, PathBuf::from(log.lines().next().unwrap()));
                }
            }
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "open exited: {}",
                fs::read_to_string(dir.join("stderr.log")).unwrap()
            );
            assert!(
                Instant::now() < deadline,
                "launcher did not complete: {}",
                fs::read_to_string(dir.join("stderr.log")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn read_place(path: &Path) -> WeakDom {
        let bytes = fs::read(path).unwrap();
        if path.extension().unwrap() == "rbxl" {
            rbx_binary::from_reader(bytes.as_slice()).unwrap()
        } else {
            rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new()).unwrap()
        }
    }
    fn marker(dom: &WeakDom) -> serde_json::Value {
        let folder = dom
            .descendants()
            .find(|node| node.name == "__RojoOpenLaunch_v1")
            .unwrap();
        assert_eq!(folder.class, "Folder");
        assert_eq!(folder.children().len(), 1);
        let value = dom.get_by_ref(folder.children()[0]).unwrap();
        assert_eq!(value.class, "StringValue");
        let Variant::String(details) = &value.properties[&"Value".into()] else {
            panic!("missing details")
        };
        serde_json::from_str(details).unwrap()
    }

    #[test]
    fn project_stem_unicode_output_and_session_marker_match_the_bound_server() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("Unicode 项目 & spaces");
        fs::create_dir(&dir).unwrap();
        let port = port();
        fixture(&dir, port);
        let output = dir.join("new place.rbxl");
        let (process, launched) = start(&dir, port, &["default", "-o", output.to_str().unwrap()]);
        assert_eq!(launched, output);
        let dom = read_place(&launched);
        let descriptor = marker(&dom);
        assert_eq!(descriptor["version"], 1);
        assert_eq!(descriptor["host"], "127.0.0.1");
        assert_eq!(descriptor["port"], port);
        assert_eq!(descriptor["projectName"], "OpenFixture");
        assert_eq!(
            descriptor["expiresAt"].as_u64().unwrap() - descriptor["createdAt"].as_u64().unwrap(),
            600
        );
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let bytes = client
            .get(format!("http://127.0.0.1:{port}/api/rojo"))
            .send()
            .unwrap()
            .bytes()
            .unwrap();
        let info: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(descriptor["sessionId"], info["sessionId"]);
        let target = dom
            .descendants()
            .find(|node| node.name == "Target")
            .unwrap();
        let pointer = dom
            .descendants()
            .find(|node| node.name == "Pointer")
            .unwrap();
        assert_eq!(
            pointer.properties.get(&"Value".into()),
            Some(&Variant::Ref(target.referent()))
        );
        drop(process);
        assert!(output.exists(), "explicit output must remain");
    }

    #[test]
    fn existing_binary_and_xml_places_are_copied_without_mutating_input() {
        for extension in ["rbxl", "rbxlx"] {
            let dir = tempfile::tempdir().unwrap();
            let port = port();
            fixture(dir.path(), port);
            let dom = WeakDom::new(
                InstanceBuilder::new("DataModel").with_child(
                    InstanceBuilder::new("Workspace").with_child(
                        InstanceBuilder::new("Part")
                            .with_name("KeepInput")
                            .with_property("Transparency", 0.25_f32),
                    ),
                ),
            );
            let input = dir.path().join(format!("input.{extension}"));
            let file = fs::File::create(&input).unwrap();
            if extension == "rbxl" {
                rbx_binary::to_writer(file, &dom, dom.root().children()).unwrap();
            } else {
                rbx_xml::to_writer(
                    file,
                    &dom,
                    dom.root().children(),
                    rbx_xml::EncodeOptions::new(),
                )
                .unwrap();
            }
            let original = fs::read(&input).unwrap();
            let output = dir.path().join(format!("copy.{extension}"));
            let (process, launched) = start(
                dir.path(),
                port,
                &[
                    input.to_str().unwrap(),
                    "--project",
                    "default",
                    "-o",
                    output.to_str().unwrap(),
                ],
            );
            assert_eq!(launched, output);
            assert_eq!(fs::read(&input).unwrap(), original);
            let copy = read_place(&output);
            let kept = copy
                .descendants()
                .find(|node| node.name == "KeepInput")
                .unwrap();
            assert_eq!(
                kept.properties.get(&"Transparency".into()),
                Some(&Variant::Float32(0.25))
            );
            marker(&copy);
            drop(process);
        }
    }

    #[test]
    fn default_output_is_a_separate_temporary_place_and_port_override_is_honored() {
        let dir = tempfile::tempdir().unwrap();
        let port = port();
        fixture(dir.path(), 12345);
        let (process, launched) = start(dir.path(), port, &["--port", &port.to_string()]);
        let parent = launched.parent().unwrap();
        assert!(parent
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("rojo-open-"));
        assert_eq!(parent.parent().unwrap(), std::env::temp_dir());
        assert_eq!(marker(&read_place(&launched))["port"], port);
        drop(process);
        // This test force-kills the command; reclaim only its exact owned artifact.
        fs::remove_file(&launched).unwrap();
        fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn bad_arguments_outputs_and_marker_collisions_never_launch_or_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let port = port();
        fixture(dir.path(), port);
        fs::write(dir.path().join("existing.rbxl"), b"keep original").unwrap();
        for args in [
            vec!["--port", "0", "-o", "new.rbxl"],
            vec!["default", "--project", "default", "-o", "new.rbxl"],
            vec!["default", "-o", "existing.rbxl"],
            vec!["existing.rbxl", "--project", "default", "-o", "new.rbxl"],
            vec!["default", "-o", "new.rbxm"],
            vec!["default", "-o", "CON.rbxl"],
        ] {
            let output = command(dir.path(), studio(), port)
                .args(args)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(!dir.path().join("launch.log").exists());
            assert!(!dir.path().join("new.rbxl").exists());
            assert_eq!(
                fs::read(dir.path().join("existing.rbxl")).unwrap(),
                b"keep original"
            );
        }
        fs::write(dir.path().join("default.project.json"), r#"{"name":"Collision","tree":{"$className":"DataModel","ReplicatedStorage":{"__RojoOpenLaunch_v1":{"$className":"Folder"}}}}"#).unwrap();
        let output = command(dir.path(), studio(), port)
            .args(["-o", "new.rbxl"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Reserved launch marker"));
        assert!(!dir.path().join("launch.log").exists());
        assert!(!dir.path().join("new.rbxl").exists());
    }

    #[test]
    fn failed_bind_and_failed_native_launch_release_server_and_keep_input() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        fixture(dir.path(), port);
        let output = command(dir.path(), studio(), port)
            .args(["-o", "new.rbxl"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Could not start the Rojo server"));
        assert!(!dir.path().join("launch.log").exists());
        assert!(!dir.path().join("new.rbxl").exists());
        drop(listener);
        let bad_install = dir.path().join("bad-studio");
        fs::create_dir_all(bad_install.join("content")).unwrap();
        fs::write(
            bad_install.join("RobloxStudioBeta.exe"),
            b"not an executable",
        )
        .unwrap();
        let output = command(dir.path(), &bad_install, port)
            .args(["-o", "new.rbxl"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Could not open Studio"));
        assert!(!dir.path().join("launch.log").exists());
        assert!(
            TcpListener::bind(("127.0.0.1", port)).is_ok(),
            "failed launch leaked its listener"
        );
        marker(&read_place(&dir.path().join("new.rbxl")));
    }
}
