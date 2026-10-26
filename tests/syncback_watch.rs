use clap::Parser;
use librojo::cli::Options;

#[test]
fn syncback_watch_accepts_explicit_repeated_writes() {
    assert!(Options::try_parse_from([
        "rojo",
        "syncback",
        "--watch",
        "-y",
        "--input",
        "input.rbxl"
    ])
    .is_ok());
}

#[test]
fn syncback_watch_accepts_read_only_preview_without_confirmation() {
    assert!(Options::try_parse_from([
        "rojo",
        "syncback",
        "--watch",
        "--dry-run",
        "--list",
        "--input",
        "input.rbxlx"
    ])
    .is_ok());
}

#[test]
fn syncback_watch_requires_explicit_repeated_write_consent() {
    let error = Options::try_parse_from(["rojo", "syncback", "--watch", "--input", "input.rbxl"])
        .unwrap_err();
    assert_eq!(error.kind(), clap::ErrorKind::MissingRequiredArgument);
}

use rbx_dom_weak::{InstanceBuilder, WeakDom};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

struct Watching(Child);
impl Drop for Watching {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn input_bytes(extension: &str, source: &str) -> Vec<u8> {
    let place = extension.starts_with("rbxl");
    let dom = WeakDom::new(
        InstanceBuilder::new(if place { "DataModel" } else { "Folder" })
            .with_name("Root")
            .with_child(
                InstanceBuilder::new("ModuleScript")
                    .with_name("Example")
                    .with_property("Source", source),
            ),
    );
    let root = [dom.root_ref()];
    let ids = if place { dom.root().children() } else { &root };
    let mut bytes = Vec::new();
    if extension.ends_with('x') {
        rbx_xml::to_writer(&mut bytes, &dom, ids, rbx_xml::EncodeOptions::new()).unwrap();
    } else {
        rbx_binary::to_writer(&mut bytes, &dom, ids).unwrap();
    }
    bytes
}

fn wait_for(watcher: &mut Watching, predicate: impl Fn() -> bool, log: &Path) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(
            watcher.0.try_wait().unwrap().is_none(),
            "watch exited: {}",
            fs::read_to_string(log).unwrap_or_default()
        );
        assert!(
            Instant::now() < deadline,
            "watch timed out: {}\nproject: {}\nnext: {:?}",
            fs::read_to_string(log).unwrap_or_default(),
            fs::read_to_string(log.parent().unwrap().join("default.project.json"))
                .unwrap_or_default(),
            fs::read_dir(log.parent().unwrap().join("next")).map(|entries| entries
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>())
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn start(project: &Path, input: &Path, log: &Path, extra: &[&str]) -> Watching {
    let stderr = fs::File::create(log).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_rojo"))
        .arg("syncback")
        .arg(project)
        .arg("--input")
        .arg(input)
        .arg("--watch")
        .args(extra)
        .stdout(fs::File::create(log.with_extension("stdout.log")).unwrap())
        .stderr(stderr)
        .spawn()
        .unwrap();
    Watching(child)
}

fn project(dir: &Path, extension: &str) -> (PathBuf, PathBuf, PathBuf) {
    fs::create_dir(dir.join("src")).unwrap();
    let project = dir.join("default.project.json");
    fs::write(&project,serde_json::to_vec(&serde_json::json!({
        "name":"Root","tree":{"$className":if extension.starts_with("rbxl") {"DataModel"} else {"Folder"},"$path":"src"}
    })).unwrap()).unwrap();
    (
        project,
        dir.join(format!("input.{extension}")),
        dir.join("watch.log"),
    )
}

#[test]
fn syncback_watch_tracks_saves_replacements_and_recovery_in_all_four_formats() {
    for extension in ["rbxm", "rbxmx", "rbxl", "rbxlx"] {
        let dir = tempfile::tempdir().unwrap();
        let (project, input, log) = project(dir.path(), extension);
        fs::write(&input, input_bytes(extension, "return 1")).unwrap();
        let mut watcher = start(&project, &input, &log, &["-y"]);
        let output = dir.path().join("src/Example.luau");
        let has_source =
            |source: &str| fs::read_to_string(&output).is_ok_and(|contents| contents == source);
        wait_for(&mut watcher, || has_source("return 1"), &log);
        wait_for(
            &mut watcher,
            || {
                fs::read_to_string(&log)
                    .unwrap_or_default()
                    .contains("Watching syncback input")
            },
            &log,
        );
        fs::write(&input, input_bytes(extension, "return 2")).unwrap();
        wait_for(&mut watcher, || has_source("return 2"), &log);
        let replacement = input.with_extension("pending");
        fs::write(&replacement, input_bytes(extension, "return 3")).unwrap();
        fs::rename(&replacement, &input).unwrap();
        wait_for(&mut watcher, || has_source("return 3"), &log);
        fs::remove_file(&input).unwrap();
        wait_for(
            &mut watcher,
            || {
                fs::read_to_string(&log)
                    .unwrap_or_default()
                    .contains("waiting for a later save")
            },
            &log,
        );
        assert!(has_source("return 3"));
        fs::write(&input, b"partial save").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(has_source("return 3"));
        fs::write(&input, input_bytes(extension, "return 4")).unwrap();
        wait_for(&mut watcher, || has_source("return 4"), &log);
        // The watch must reload source ownership and project edits on each pass.
        fs::create_dir(dir.path().join("next")).unwrap();
        let text = fs::read_to_string(&project).unwrap().replace("src", "next");
        fs::write(&project, text).unwrap();
        fs::write(&input, input_bytes(extension, "return 5")).unwrap();
        let next = dir.path().join("next/Example.luau");
        wait_for(
            &mut watcher,
            || fs::read_to_string(&next).is_ok_and(|value| value == "return 5"),
            &log,
        );
        let before = fs::read_to_string(&log).unwrap();
        let passes = before.matches("Beginning syncback").count();
        assert_eq!(passes, 5, "{extension}: {before}");
        let same = fs::read(&input).unwrap();
        fs::write(&input, same).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let after = fs::read_to_string(&log).unwrap();
        assert_eq!(
            after.matches("Beginning syncback").count(),
            passes,
            "duplicate input or output feedback loop"
        );
    }
}

#[test]
fn syncback_watch_dry_run_lists_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let (project, input, log) = project(dir.path(), "rbxm");
    fs::write(&input, input_bytes("rbxm", "return 1")).unwrap();
    let mut watcher = start(&project, &input, &log, &["--dry-run", "--list"]);
    wait_for(
        &mut watcher,
        || {
            fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("Watching syncback input")
        },
        &log,
    );
    assert_eq!(fs::read_dir(dir.path().join("src")).unwrap().count(), 0);
    assert!(fs::read_to_string(log.with_extension("stdout.log"))
        .unwrap()
        .contains("Example.luau"));
    fs::write(&input, input_bytes("rbxm", "return 2")).unwrap();
    wait_for(
        &mut watcher,
        || {
            fs::read_to_string(&log)
                .unwrap_or_default()
                .matches("Beginning syncback")
                .count()
                == 2
        },
        &log,
    );
    assert_eq!(fs::read_dir(dir.path().join("src")).unwrap().count(), 0);
}

#[test]
fn syncback_watch_rejects_input_removed_by_its_own_output_plan() {
    for mode in ["-y", "--dry-run"] {
        let dir = tempfile::tempdir().unwrap();
        let (project, _, _) = project(dir.path(), "rbxm");
        let input = dir.path().join("src/Watched.rbxm");
        let bytes = input_bytes("rbxm", "return 42");
        fs::write(&input, &bytes).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_rojo"))
            .arg("syncback")
            .arg(project)
            .arg("--input")
            .arg(&input)
            .arg("--watch")
            .arg(mode)
            .output()
            .unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("overwrite watched input"), "{error}");
        assert_eq!(fs::read(&input).unwrap(), bytes);
        assert!(!dir.path().join("src/Example.luau").exists());
    }
}

#[cfg(windows)]
#[test]
fn syncback_watch_stops_on_output_write_failure_instead_of_retrying_partial_writes() {
    struct PermissionsGuard(PathBuf, fs::Permissions);
    impl Drop for PermissionsGuard {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, self.1.clone());
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let (project, input, log) = project(dir.path(), "rbxm");
    fs::write(&input, input_bytes("rbxm", "return 1")).unwrap();
    let mut watcher = start(&project, &input, &log, &["-y"]);
    let output = dir.path().join("src/Example.luau");
    wait_for(
        &mut watcher,
        || {
            fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("Watching syncback input")
        },
        &log,
    );
    let original = fs::metadata(&output).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    let _restore = PermissionsGuard(output.clone(), original);
    fs::set_permissions(&output, readonly).unwrap();
    fs::write(&input, input_bytes("rbxm", "return 2")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = watcher.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "write failure did not stop watch"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(!status.success());
    assert!(fs::read_to_string(&log)
        .unwrap()
        .contains("Could not write syncback output"));
    assert_eq!(fs::read_to_string(output).unwrap(), "return 1");
}
