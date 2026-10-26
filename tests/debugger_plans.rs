use clap::Parser;
use librojo::cli::Options;

#[test]
fn debugger_plans_accept_explicit_serve_flag() {
    assert!(Options::try_parse_from(["rojo", "serve", "--debugger-plans"]).is_ok());
    assert!(Options::try_parse_from(["rojo", "serve"]).is_ok());
}

#[test]
fn debugger_plans_refuse_non_loopback_binding_before_listening() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("default.project.json"),
        r#"{"name":"Debug","tree":{"$className":"Folder"}}"#,
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rojo"))
        .args(["serve", "--debugger-plans", "--address", "0.0.0.0"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("loopback"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("listening"));
}
