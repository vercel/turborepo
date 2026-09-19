#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! Integration tests for `turbo setup`.
//!
//! Downloads are served by an in-process mock registry so the tests stay
//! offline and deterministic. The fake `npm` package publishes a real `bin`
//! entry so the test can prove that `turbo run` picks the installed tool up
//! through `.turbo/tools/bin` without any changes to the caller's `PATH`.

mod common;

use std::{fs, io::Write, path::Path};

use common::{run_turbo, run_turbo_with_env, setup};
use httpmock::prelude::*;

const NPM_VERSION: &str = "10.5.0";

/// A gzip'd npm tarball (`package/…`) for a fake `npm` whose CLI prints
/// its arguments and exits 0.
fn fake_npm_tarball() -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut add = |path: &str, contents: &str, mode: u32| {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        builder
            .append_data(&mut header, path, contents.as_bytes())
            .unwrap();
    };
    add(
        "package/package.json",
        &format!(
            r#"{{"name":"npm","version":"{NPM_VERSION}","bin":{{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}}}"#
        ),
        0o644,
    );
    let cli = "#!/usr/bin/env node\nconsole.log('fake npm ' + process.argv.slice(2).join(' '));\n";
    add("package/bin/npm-cli.js", cli, 0o755);
    add("package/bin/npx-cli.js", cli, 0o755);
    builder.into_inner().unwrap().finish().unwrap()
}

/// Serves the metadata and tarball for `npm@10.5.0` the way the npm registry
/// does, without an `integrity` field so no real hash is needed.
fn mock_registry(server: &MockServer) {
    let tarball_url = server.url(format!("/npm/-/npm-{NPM_VERSION}.tgz"));
    server.mock(|when, then| {
        when.method(GET).path(format!("/npm/{NPM_VERSION}"));
        then.status(200)
            .header("content-type", "application/json")
            .body(
                serde_json::json!({
                    "name": "npm",
                    "version": NPM_VERSION,
                    "bin": {"npm": "bin/npm-cli.js", "npx": "bin/npx-cli.js"},
                    "dist": {"tarball": tarball_url}
                })
                .to_string(),
            );
    });
    server.mock(|when, then| {
        when.method(GET)
            .path(format!("/npm/-/npm-{NPM_VERSION}.tgz"));
        then.status(200).body(fake_npm_tarball());
    });
}

fn remove_json_field(dir: &Path, field: &str) {
    let pkg_path = dir.join("package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg.as_object_mut().unwrap().remove(field);
    let mut file = fs::File::create(&pkg_path).unwrap();
    writeln!(file, "{}", serde_json::to_string_pretty(&pkg).unwrap()).unwrap();
}

#[test]
fn test_setup_errors_without_declarations() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    remove_json_field(tempdir.path(), "packageManager");

    let output = run_turbo(tempdir.path(), &["setup"]);
    assert!(
        !output.status.success(),
        "setup must fail without declarations"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No toolchain declarations found"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("rust-toolchain.toml") && stderr.contains("go.work"),
        "error should list the files turbo setup reads: {stderr}"
    );
    assert!(
        !tempdir.path().join(".turbo/tools").exists(),
        "nothing should be created on failure"
    );
}

#[test]
fn test_setup_check_reports_missing_tools() {
    let server = MockServer::start();
    mock_registry(&server);
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["setup", "--check"],
        &[("TURBO_TOOLS_NPM_REGISTRY", &server.base_url())],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "--check exits 1 when tools are missing"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("npm 10.5.0"), "{stdout}");
    assert!(stdout.contains("missing"), "{stdout}");
    assert!(
        !tempdir.path().join(".turbo/tools/bin").exists(),
        "--check must not install anything"
    );
}

#[cfg(not(windows))]
#[test]
fn test_setup_installs_package_manager_and_tasks_use_it() {
    let server = MockServer::start();
    mock_registry(&server);
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    let env = [("TURBO_TOOLS_NPM_REGISTRY", server.base_url())];
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let output = run_turbo_with_env(tempdir.path(), &["setup"], &env);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("npm 10.5.0"), "{stdout}");
    assert!(
        stdout.contains("(from package.json#packageManager)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("1 tool installed into .turbo/tools"),
        "{stdout}"
    );

    let tools = tempdir.path().join(".turbo/tools");
    assert!(tools.join("bin/npm").exists(), "npm shim should exist");
    assert!(tools.join("bin/npx").exists(), "npx shim should exist");
    assert!(tools.join("npm/10.5.0/bin/npm-cli.js").exists());
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(tools.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["tools"]["npm"]["version"], NPM_VERSION);
    assert_eq!(manifest["tools"]["npm"]["path"], "npm/10.5.0");
    let gitignore = fs::read_to_string(tempdir.path().join(".gitignore")).unwrap_or_default();
    assert!(gitignore.lines().any(|line| line.trim() == ".turbo"));

    // A second run is a no-op.
    let output = run_turbo_with_env(tempdir.path(), &["setup"], &env);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("up to date"), "{stdout}");
    assert!(stdout.contains("0 tools installed"), "{stdout}");

    let output = run_turbo_with_env(tempdir.path(), &["setup", "--check"], &env);
    assert!(output.status.success(), "--check passes after install");

    // `turbo info` surfaces the managed tools.
    let output = run_turbo(tempdir.path(), &["info"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Managed tools (.turbo/tools): npm 10.5.0"),
        "{stdout}"
    );

    // Tasks resolve the package manager from .turbo/tools/bin even though the
    // caller's PATH never changed: the fake npm prints instead of building.
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=full"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("fake npm run build"),
        "task should run through the installed npm: {stdout}"
    );
}
