mod common;

use std::{path::Path, process::Command};

use common::setup;

fn run_turbo_from(dir: &Path, args: &[&str]) -> std::process::Output {
    let config_dir = tempfile::tempdir().unwrap();
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").unwrap();
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(dir);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().unwrap()
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn setup_git_in(dir: &Path) {
    setup::setup_git(dir).unwrap();
}

fn install_deps_in(dir: &Path) {
    let npm = which::which("npm").unwrap_or_else(|_| "npm".into());
    Command::new(npm)
        .args(["install", "--silent"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Install dependencies", "--quiet"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
}

// --- has-workspaces.t ---

#[test]
fn test_has_workspaces() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "inference/has_workspaces",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    // From root: no pkg_inference_root
    let output = run_turbo_from(tempdir.path(), &["run", "build", "--filter=nothing", "-vv"]);
    let out = combined_output(&output);
    assert!(
        !out.contains("pkg_inference_root set"),
        "root should not set pkg_inference_root"
    );
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // From apps/web
    let output = run_turbo_from(
        &tempdir.path().join("apps/web"),
        &["run", "build", "--filter=nothing", "-vv"],
    );
    let out = combined_output(&output);
    assert!(out.contains("pkg_inference_root set to \"apps") && out.contains("web\""));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // From crates
    let output = run_turbo_from(
        &tempdir.path().join("crates"),
        &["run", "build", "--filter=nothing", "-vv"],
    );
    let out = combined_output(&output);
    assert!(out.contains("pkg_inference_root set to \"crates\""));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // From crates/super-crate/tests/test-package
    let output = run_turbo_from(
        &tempdir.path().join("crates/super-crate/tests/test-package"),
        &["run", "build", "--filter=nothing", "-vv"],
    );
    let out = combined_output(&output);
    assert!(out.contains("pkg_inference_root set to \"crates") && out.contains("test-package\""));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // From packages/ui-library/src
    let output = run_turbo_from(
        &tempdir.path().join("packages/ui-library/src"),
        &["run", "build", "--filter=nothing", "-vv"],
    );
    let out = combined_output(&output);
    assert!(out.contains("pkg_inference_root set to \"packages") && out.contains("src\""));
    assert!(out.contains("No package found with name 'nothing' in workspace"));
}

// --- has-workspaces-dot-prefix.t ---

#[test]
fn test_has_workspaces_dot_prefix() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "inference/has_workspaces_dot_prefix",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    // From apps/web: should detect monorepo
    let output = run_turbo_from(&tempdir.path().join("apps/web"), &["run", "build"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("web:build:"),
        "should run as monorepo with task prefix"
    );
    assert!(stdout.contains("1 successful, 1 total"));

    // Filter by package name
    let output = run_turbo_from(tempdir.path(), &["run", "build", "--filter=ui"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ui:build:"));
    assert!(stdout.contains("1 successful, 1 total"));

    // Filter with "./" prefix path
    let output = run_turbo_from(tempdir.path(), &["run", "build", "--filter=./packages/ui"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ui:build:"));
    assert!(stdout.contains("1 successful, 1 total"));

    // Filter with "./" prefix for apps/web
    let output = run_turbo_from(tempdir.path(), &["run", "build", "--filter=./apps/web"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("web:build:"));
    assert!(stdout.contains("1 successful, 1 total"));
}

// --- no-workspaces.t ---

#[test]
fn test_no_workspaces() {
    let tempdir = tempfile::tempdir().unwrap();
    let target = tempdir.path().join("no_workspaces");
    std::fs::create_dir_all(&target).unwrap();

    // Replicate no_workspaces_setup.sh
    setup::copy_fixture("inference/no_workspaces", &target).unwrap();
    setup_git_in(&target);
    install_deps_in(&target);

    setup_git_in(&target.join("parent"));
    install_deps_in(&target.join("parent"));

    setup_git_in(&target.join("parent/child"));
    install_deps_in(&target.join("parent/child"));

    // Run from root
    let output = run_turbo_from(&target, &["run", "build", "--filter=nothing"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No package found with name 'nothing' in workspace"));

    // Run from parent
    let output = run_turbo_from(
        &target.join("parent"),
        &["run", "build", "--filter=nothing"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No package found with name 'nothing' in workspace"));

    // Run from parent/child
    let output = run_turbo_from(
        &target.join("parent/child"),
        &["run", "build", "--filter=nothing"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No package found with name 'nothing' in workspace"));
}

// --- nested-workspaces.t ---

#[test]
fn test_nested_workspaces() {
    let tempdir = tempfile::tempdir().unwrap();
    let target = tempdir.path().join("nested_workspaces");
    std::fs::create_dir_all(&target).unwrap();

    // Replicate nested_workspaces_setup.sh
    setup::copy_fixture("inference/nested_workspaces", &target).unwrap();

    for subdir in [
        "outer",
        "outer/inner",
        "outer/inner-no-turbo",
        "outer-no-turbo",
        "outer-no-turbo/inner",
        "outer-no-turbo/inner-no-turbo",
    ] {
        setup_git_in(&target.join(subdir));
        install_deps_in(&target.join(subdir));
    }

    // Helper: run turbo from a subdir, return combined stdout+stderr
    let run = |subdir: &str| -> String {
        let output = run_turbo_from(
            &target.join(subdir),
            &["run", "build", "--filter=nothing", "-vv"],
        );
        combined_output(&output)
    };

    // outer: finds repo root at outer
    let out = run("outer");
    assert!(out.contains("nested_workspaces") && out.contains("outer"));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // outer/apps: still finds outer as root
    let out = run("outer/apps");
    assert!(out.contains("nested_workspaces") && out.contains("outer"));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // outer/inner: finds inner as root
    let out = run("outer/inner");
    assert!(out.contains("outer") && out.contains("inner"));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // outer/inner/apps: finds inner as root
    let out = run("outer/inner/apps");
    assert!(out.contains("outer") && out.contains("inner"));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // outer/inner-no-turbo: finds root but no turbo.json
    let out = run("outer/inner-no-turbo");
    assert!(out.contains("inner-no-turbo"));
    assert!(out.contains("Could not find turbo.json"));

    // outer/inner-no-turbo/apps
    let out = run("outer/inner-no-turbo/apps");
    assert!(out.contains("inner-no-turbo"));
    assert!(out.contains("Could not find turbo.json"));

    // outer-no-turbo: no turbo.json
    let out = run("outer-no-turbo");
    assert!(out.contains("outer-no-turbo"));
    assert!(out.contains("Could not find turbo.json"));

    // outer-no-turbo/apps
    let out = run("outer-no-turbo/apps");
    assert!(out.contains("outer-no-turbo"));
    assert!(out.contains("Could not find turbo.json"));

    // outer-no-turbo/inner: HAS turbo.json
    let out = run("outer-no-turbo/inner");
    assert!(out.contains("outer-no-turbo") && out.contains("inner"));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // outer-no-turbo/inner/apps
    let out = run("outer-no-turbo/inner/apps");
    assert!(out.contains("outer-no-turbo") && out.contains("inner"));
    assert!(out.contains("No package found with name 'nothing' in workspace"));

    // outer-no-turbo/inner-no-turbo: no turbo.json
    let out = run("outer-no-turbo/inner-no-turbo");
    assert!(out.contains("inner-no-turbo"));
    assert!(out.contains("Could not find turbo.json"));

    // outer-no-turbo/inner-no-turbo/apps
    let out = run("outer-no-turbo/inner-no-turbo/apps");
    assert!(out.contains("inner-no-turbo"));
    assert!(out.contains("Could not find turbo.json"));
}
