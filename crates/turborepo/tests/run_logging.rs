#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{
    git, git_commit_staged_if_changed, run_turbo, run_turbo_with_env, setup, turbo_command,
};

#[test]
fn test_github_actions_grouped_logging_smoke() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &[
            "run",
            "build",
            "--force",
            "--log-prefix=task",
            "--filter=util",
        ],
        &[("GITHUB_ACTIONS", "1")],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("::group::util:build"));
    assert!(stdout.contains("util:build: cache bypass"));
    assert!(stdout.contains("::endgroup::"));

    let output = run_turbo_with_env(tempdir.path(), &["run", "fail"], &[("GITHUB_ACTIONS", "1")]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("::error::"),
        "stderr should contain ::error:: annotation for GitHub Actions, got: {}",
        &stderr[..stderr.len().min(500)]
    );
}

// --- log-prefix.t ---

#[test]
fn test_log_prefix_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    assert!(stdout.contains("build-app-a"));
    assert!(!stdout.contains("app-a:build:"));

    // Check cached log file doesn't have prefixes
    let log = std::fs::read_to_string(tempdir.path().join("app-a/.turbo/turbo-build.log")).unwrap();
    assert!(log.contains("build-app-a"));
    assert!(!log.contains("app-a:build:"));

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, replaying logs"));
    assert!(stdout.contains("FULL TURBO"));
    assert!(!stdout.contains("app-a:build:"));
}

// --- https://github.com/vercel/turborepo/issues/15033 ---

#[test]
fn test_no_color_strips_task_output_live_and_replayed() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup::setup_integration_test(dir, "run_logging", "npm@10.5.0", false).unwrap();
    let paint = "node -e \"process.stdout.write('color=' + (process.env.FORCE_COLOR ? \
                 '\\x1b[31mred\\x1b[39m' : 'plain') + '\\n')\"";
    std::fs::write(
        dir.join("app-a/package.json"),
        serde_json::json!({ "name": "app-a", "scripts": { "paint": paint } }).to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("turbo.json"), r#"{ "tasks": { "paint": {} } }"#).unwrap();
    git(dir, &["add", "."]);
    git_commit_staged_if_changed(dir, "add paint task");

    let paint_line = |extra_args: &[&str], env: &[(&str, &str)]| {
        let mut cmd = turbo_command(dir);
        cmd.env_remove("FORCE_COLOR").env_remove("NO_COLOR");
        cmd.args(["run", "paint", "--filter=app-a", "--ui=stream"]);
        cmd.args(extra_args).envs(env.iter().copied());
        let output = cmd.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let line = stdout
            .lines()
            .find(|line| line.contains("color=") && !line.contains("node -e"))
            .unwrap_or_else(|| panic!("no color= line in {stdout}"))
            .to_owned();
        (stdout.contains("cache hit, replaying logs"), line)
    };

    let (hit, line) = paint_line(&[], &[("FORCE_COLOR", "3")]);
    assert!(!hit);
    assert!(line.ends_with("color=\x1b[31mred\x1b[39m"), "{line:?}");

    let (hit, line) = paint_line(&["--no-color"], &[]);
    assert!(hit);
    assert_eq!(line, "app-a:paint: color=red");

    let (hit, line) = paint_line(&[], &[("NO_COLOR", "1")]);
    assert!(hit);
    assert_eq!(line, "app-a:paint: color=red");

    // Without a request for no color, the replay keeps the recorded codes.
    let (hit, line) = paint_line(&[], &[]);
    assert!(hit);
    assert!(line.ends_with("color=\x1b[31mred\x1b[39m"), "{line:?}");

    // Live output is stripped too, even when the task emits color.
    let (hit, line) = paint_line(&["--no-color", "--force"], &[("FORCE_COLOR", "3")]);
    assert!(!hit);
    assert_eq!(line, "app-a:paint: color=red");
}
