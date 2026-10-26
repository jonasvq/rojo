use std::{fs, process::Command};

#[test]
fn build_can_select_zstd_binary_compression() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("default.project.json"),
        r#"{"name":"Compressed","tree":{"$className":"Folder","Data":{"$className":"StringValue","$properties":{"Value":"compress this repeated data compress this repeated data"}}}}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args(["build", "--compress", "zstd", "-o", "out.rbxm"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(dir.path().join("out.rbxm")).unwrap();
    assert!(bytes
        .windows(4)
        .any(|window| window == [0x28, 0xb5, 0x2f, 0xfd]));
    let dom = rbx_binary::from_reader(bytes.as_slice()).unwrap();
    assert!(dom.descendants().any(|node| node.name == "Compressed"));
}

fn assert_chunks(bytes: &[u8], mode: &str) {
    let mut position = 32;
    let mut data_chunks = 0;
    while position < bytes.len() {
        let header = &bytes[position..position + 16];
        let compressed = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let raw = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        let length = if compressed == 0 { raw } else { compressed };
        let payload = &bytes[position + 16..position + 16 + length];
        if &header[..4] != b"END\0" {
            data_chunks += 1;
            if mode == "none" {
                assert_eq!(compressed, 0);
            } else {
                assert!(compressed > 0);
                assert_eq!(
                    payload.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]),
                    mode == "zstd"
                );
            }
        }
        position += 16 + length;
    }
    assert!(data_chunks > 0);
    assert_eq!(position, bytes.len());
}

fn success(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_compression_preserves_model_place_references_and_legacy_physics() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("default.project.json"),
        r#"{
        "name":"Root","tree":{"$className":"DataModel",
        "Workspace":{"$className":"Workspace",
            "Old":{"$className":"MeshPart","$attributes":{"Rojo_Id":"old"}},
            "Modern":{"$className":"MeshPart","$properties":{"UnscaledVolume":4}},
            "Model":{"$className":"Model","$properties":{"NeedsPivotMigration":false}},
            "Pointer":{"$className":"ObjectValue","$attributes":{"Rojo_Target_Value":"old"}}}}}
        "#,
    )
    .unwrap();
    for extension in ["rbxm", "rbxl"] {
        let output = format!("out.{extension}");
        success(dir.path(), &["build", "-o", &output]);
        let default_bytes = fs::read(dir.path().join(&output)).unwrap();
        for mode in ["lz4", "zstd", "none"] {
            success(dir.path(), &["build", "--compress", mode, "-o", &output]);
            let bytes = fs::read(dir.path().join(&output)).unwrap();
            assert_chunks(&bytes, mode);
            if mode == "lz4" {
                assert_eq!(bytes, default_bytes);
            }
            let dom = rbx_binary::from_reader(bytes.as_slice()).unwrap();
            assert_eq!(
                dom.descendants().any(|node| node.name == "Root"),
                extension == "rbxm"
            );
            let old = dom.descendants().find(|node| node.name == "Old").unwrap();
            assert_eq!(
                old.properties.get(&"UnscaledVolume".into()),
                Some(&rbx_dom_weak::types::Variant::Float32(0.0))
            );
            let modern = dom
                .descendants()
                .find(|node| node.name == "Modern")
                .unwrap();
            assert_eq!(
                modern.properties.get(&"UnscaledVolume".into()),
                Some(&rbx_dom_weak::types::Variant::Float32(4.0))
            );
            let pointer = dom
                .descendants()
                .find(|node| node.name == "Pointer")
                .unwrap();
            assert_eq!(
                pointer.properties.get(&"Value".into()),
                Some(&rbx_dom_weak::types::Variant::Ref(old.referent()))
            );
            let model = dom.descendants().find(|node| node.name == "Model").unwrap();
            assert_eq!(
                model.properties.get(&"NeedsPivotMigration".into()),
                Some(&rbx_dom_weak::types::Variant::Bool(false))
            );
        }
    }
}

#[test]
fn invalid_compression_and_xml_requests_preserve_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("default.project.json"),
        r#"{"name":"Root","tree":{"$className":"DataModel"}}"#,
    )
    .unwrap();
    for (extension, mode) in [
        ("rbxm", "invalid"),
        ("rbxmx", "lz4"),
        ("rbxlx", "zstd"),
        ("rbxmx", "none"),
    ] {
        let output = format!("out.{extension}");
        fs::write(dir.path().join(&output), b"keep old output").unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", "--compress", mode, "-o", &output])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("compress"));
        assert_eq!(
            fs::read(dir.path().join(&output)).unwrap(),
            b"keep old output"
        );
    }
    success(dir.path(), &["build", "-o", "base.rbxl"]);
    fs::write(dir.path().join("out.rbxlx"), b"keep XML output").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .current_dir(dir.path())
        .args([
            "build",
            "--input",
            "base.rbxl",
            "--compress",
            "zstd",
            "-o",
            "out.rbxlx",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        fs::read(dir.path().join("out.rbxlx")).unwrap(),
        b"keep XML output"
    );
}

#[test]
fn stdin_and_same_path_overlay_honor_selected_compression() {
    use std::{io::Write, process::Stdio};
    let dir = tempfile::tempdir().unwrap();
    let project = br#"{"name":"Root","tree":{"$className":"DataModel","Workspace":{"$className":"Workspace","Child":{"$className":"Folder"}}}}"#;
    fs::write(dir.path().join("default.project.json"), project).unwrap();
    success(dir.path(), &["build", "-o", "base.rbxl"]);
    for mode in ["zstd", "none", "lz4"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args([
                "build",
                "-",
                "--input",
                "base.rbxl",
                "--compress",
                mode,
                "-o",
                "base.rbxl",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(project).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = fs::read(dir.path().join("base.rbxl")).unwrap();
        assert_chunks(&bytes, mode);
        let dom = rbx_binary::from_reader(bytes.as_slice()).unwrap();
        assert!(dom.descendants().any(|node| node.name == "Child"));
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn binary_watch_uses_zstd_for_initial_and_later_exports() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    struct Watching(std::process::Child);
    impl Drop for Watching {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Source.lua"), "return 1").unwrap();
    fs::write(
        dir.path().join("default.project.json"),
        r#"{"name":"Root","tree":{"$path":"Source.lua"}}"#,
    )
    .unwrap();
    let mut child = Watching(
        Command::new(env!("CARGO_BIN_EXE_rojo"))
            .current_dir(dir.path())
            .args(["build", "--watch", "--compress", "zstd", "-o", "out.rbxm"])
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.path().join("stderr.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    for source in ["return 1", "return 222"] {
        fs::write(dir.path().join("Source.lua"), source).unwrap();
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Ok(bytes) = fs::read(dir.path().join("out.rbxm")) {
                if let Ok(dom) = rbx_binary::from_reader(bytes.as_slice()) {
                    if dom.descendants().any(|node| {
                        node.properties.get(&"Source".into())
                            == Some(&rbx_dom_weak::types::Variant::String(source.into()))
                    }) {
                        assert_chunks(&bytes, "zstd");
                        break;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "watch did not update: {}",
                fs::read_to_string(dir.path().join("stderr.log")).unwrap()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(child.0.try_wait().unwrap().is_none());
    }
}
