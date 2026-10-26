use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn run(dir: &Path, args: &[&str], bytes: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn stdin_accepts_bom_jsonc_unicode_and_directory_name_fallback() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("Überblick");
    fs::create_dir(&dir).unwrap();
    let bytes = b"\xef\xbb\xbf{// stdin JSONC\n\"tree\":{\"$className\":\"Folder\"},}";
    let output = run(&dir, &["build", "-", "-o", "out.rbxm"], bytes);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dom = rbx_binary::from_reader(fs::File::open(dir.join("out.rbxm")).unwrap()).unwrap();
    assert_eq!(
        dom.get_by_ref(dom.root().children()[0]).unwrap().name,
        "Überblick"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn invalid_stdin_and_source_errors_preserve_existing_output_and_create_no_project_file() {
    for bytes in [
        b"".as_slice(),
        b"{broken",
        br#"{"name":"Missing","tree":{"$path":"missing.lua"}}"#,
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("out.rbxmx"), "preserve existing output").unwrap();
        let output = run(dir.path(), &["build", "-", "-o", "out.rbxmx"], bytes);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("stdin"));
        assert_eq!(
            fs::read_to_string(dir.path().join("out.rbxmx")).unwrap(),
            "preserve existing output"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn stdin_uses_normal_nested_project_cycle_checks() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("cycle.project.json"),
        r#"{"name":"Cycle","tree":{"$path":"cycle.project.json"}}"#,
    )
    .unwrap();
    let output = run(
        dir.path(),
        &["build", "-", "-o", "out.rbxmx"],
        br#"{"name":"Stdin","tree":{"$path":"cycle.project.json"}}"#,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Circular project reference"));
    assert!(!dir.path().join("out.rbxmx").exists());
}

#[test]
fn stdin_can_update_an_existing_place_with_relative_sources() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("base.project.json"),r#"{"name":"Base","tree":{"$className":"DataModel","Workspace":{"$className":"Workspace","Existing":{"$className":"Folder"}}}}"#).unwrap();
    let built = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "base.project.json", "-o", "base.rbxl"])
        .output()
        .unwrap();
    assert!(built.status.success());
    fs::write(dir.path().join("Source.lua"), "return 123").unwrap();
    let output=run(dir.path(),&["build","-","--input","base.rbxl","-o","base.rbxl"],br#"{"name":"Stdin","tree":{"$className":"DataModel","Workspace":{"$className":"Workspace","$ignoreUnknownInstances":true,"New":{"$path":"Source.lua"}}}}"#);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dom =
        rbx_binary::from_reader(fs::File::open(dir.path().join("base.rbxl")).unwrap()).unwrap();
    assert!(dom.descendants().any(|node| node.name == "Existing"));
    assert!(dom.descendants().any(|node| node.name == "New"
        && node.properties.get(&"Source".into())
            == Some(&rbx_dom_weak::types::Variant::String("return 123".into()))));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[test]
fn stdin_watch_retains_the_frozen_root_and_tracks_real_source_changes() {
    struct Watching(std::process::Child);
    impl Drop for Watching {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/Thing.lua"), "return 1").unwrap();
    fs::write(
        dir.path().join("default.project.json"),
        "invalid physical project must not be read",
    )
    .unwrap();
    let mut child = Watching(
        Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", "-", "--watch", "-o", "out.rbxmx"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.path().join("stderr.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    child
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"name":"FrozenStdin","tree":{"$className":"Folder","$path":"src"}}"#)
        .unwrap();
    let wait_for = |text: &str| {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if fs::read_to_string(dir.path().join("out.rbxmx"))
                .is_ok_and(|xml| xml.contains(text) && xml.contains("FrozenStdin"))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "watch failed for {text}: {}",
                fs::read_to_string(dir.path().join("stderr.log")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for("return 1");
    fs::write(dir.path().join("src/New.lua"), "return 'new child'").unwrap();
    wait_for("return 'new child'");
    fs::write(dir.path().join("src/Thing.lua"), "return 999").unwrap();
    wait_for("return 999");
    fs::remove_file(dir.path().join("src/New.lua")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Ok(bytes) = fs::read(dir.path().join("out.rbxmx")) {
            if let Ok(dom) = rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new()) {
                let nodes: Vec<_> = dom.descendants().collect();
                if nodes.iter().any(|node| node.name == "FrozenStdin")
                    && nodes.iter().any(|node| {
                        node.name == "Thing"
                            && node.properties.get(&"Source".into())
                                == Some(&rbx_dom_weak::types::Variant::String("return 999".into()))
                    })
                    && !nodes.iter().any(|node| node.name == "New")
                {
                    break;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "deleted stdin child remained or output invalid"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(child.0.try_wait().unwrap().is_none());
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        4,
        "no temporary root project may be written"
    );
}

#[test]
fn build_reads_project_json_from_stdin_without_a_temporary_project_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/Thing.lua"), "return 42").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "-", "--output", "out.rbxmx"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(br#"{"name":"FromStdin","tree":{"$className":"Folder","Thing":{"$path":"src/Thing.lua"}}}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let xml = fs::read_to_string(dir.path().join("out.rbxmx")).unwrap();
    assert!(xml.contains("FromStdin") && xml.contains("return 42"));
    let mut files: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec!["out.rbxmx", "src"],
        "stdin must not create a project file"
    );
}

#[test]
fn normal_project_builds_ignore_stdin_and_keep_default_resolution() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("default.project.json"),
        r#"{"name":"PhysicalProject","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    for args in [
        vec!["build", "-o", "out.rbxmx"],
        vec!["build", "default", "-o", "out.rbxmx"],
    ] {
        let output = run(dir.path(), &args, b"invalid and deliberately unused stdin");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(fs::read_to_string(dir.path().join("out.rbxmx"))
            .unwrap()
            .contains("PhysicalProject"));
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
