#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{combined_output, run_turbo, setup};

fn setup_cargo_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("crates/app/src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n\n[workspace.metadata]\nname = \
         \"acme\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("crates/app/Cargo.toml"),
        "[package]\nname = \"cargo-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(root.join("crates/app/src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(
        root.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"cargo-app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("turbo.json"),
        r#"{"futureFlags":{"experimentalCargoWorkspaces":true},"tasks":{"build":{}}}"#,
    )
    .unwrap();
}

fn config_json(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("expected valid JSON from turbo config")
}

#[test]
fn test_config_defaults() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["config"]);
    assert!(output.status.success());

    let cfg = config_json(&output);
    assert_eq!(cfg["apiUrl"], "https://vercel.com/api");
    assert_eq!(cfg["loginUrl"], "https://vercel.com");
    assert!(cfg["teamSlug"].is_null());
    assert_eq!(cfg["signature"], false);
    assert_eq!(cfg["timeout"], 30);
    assert_eq!(cfg["uploadTimeout"], 60);
    assert_eq!(cfg["envMode"], "strict");
    assert!(cfg["daemon"].is_null());
    assert!(cfg["concurrency"].is_null());
    assert_eq!(cfg["packageManager"], "npm");
}

#[test]
fn config_supports_pure_cargo_without_changing_protocol_shape() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_workspace(tempdir.path());

    let output = run_turbo(tempdir.path(), &["config"]);
    assert!(output.status.success(), "{output:?}");
    assert!(config_json(&output).get("packageManager").is_none());
}

#[test]
fn config_preserves_javascript_package_manager_output() {
    let tempdir = tempfile::tempdir().unwrap();
    std::fs::write(
        tempdir.path().join("package.json"),
        r#"{"name":"root","packageManager":"npm@10.5.0"}"#,
    )
    .unwrap();
    std::fs::write(
        tempdir.path().join("package-lock.json"),
        r#"{"name":"root","lockfileVersion":3,"packages":{"":{"name":"root"}}}"#,
    )
    .unwrap();
    std::fs::write(tempdir.path().join("turbo.json"), r#"{"tasks":{}}"#).unwrap();

    let output = run_turbo(tempdir.path(), &["config"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(config_json(&output)["packageManager"], "npm");
}

#[test]
fn config_introspects_invalid_run_concurrency_without_validating_it() {
    let tempdir = tempfile::tempdir().unwrap();
    std::fs::write(
        tempdir.path().join("package.json"),
        r#"{"name":"root","packageManager":"npm@10.5.0"}"#,
    )
    .unwrap();
    std::fs::write(
        tempdir.path().join("package-lock.json"),
        r#"{"name":"root","lockfileVersion":3,"packages":{"":{"name":"root"}}}"#,
    )
    .unwrap();
    std::fs::write(tempdir.path().join("turbo.json"), r#"{"tasks":{}}"#).unwrap();

    let output = run_turbo(tempdir.path(), &["--concurrency=invalid", "config"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(config_json(&output)["concurrency"], "invalid");
}

#[test]
fn config_rejects_missing_root_manifest_without_cargo() {
    let tempdir = tempfile::tempdir().unwrap();
    std::fs::write(tempdir.path().join("turbo.json"), r#"{"tasks":{}}"#).unwrap();

    let output = run_turbo(tempdir.path(), &["config"]);
    let combined = combined_output(&output);
    assert!(!output.status.success(), "config unexpectedly succeeded");
    assert!(
        combined.contains("Unable to read package.json"),
        "expected missing package.json diagnostic, got: {combined}"
    );
}

#[test]
fn config_rejects_malformed_root_manifest_with_cargo_enabled() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_workspace(tempdir.path());
    std::fs::write(tempdir.path().join("package.json"), "{").unwrap();

    let output = run_turbo(tempdir.path(), &["config"]);
    let combined = combined_output(&output);
    assert!(!output.status.success(), "config unexpectedly succeeded");
    assert!(
        combined.contains("Unable to parse package.json"),
        "expected package.json parse diagnostic, got: {combined}"
    );
}
