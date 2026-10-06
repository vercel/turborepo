#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, time::Duration};

use common::{combined_output, run_turbo, run_turbo_with_env, setup};
use turborepo_vercel_api_mock::{request_open_port, start_test_server};

/// Starts the mock Vercel API on its own runtime so it keeps serving while
/// the test blocks on `turbo` subprocesses.
fn start_mock_api() -> String {
    let port = request_open_port().unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let (server_ready_tx, server_ready_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(start_test_server(port, Some(server_ready_tx)));
            server_ready_rx.await.unwrap();
            ready_tx.send(()).unwrap();
            server.await.unwrap().unwrap();
        });
    });
    ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    format!("http://localhost:{port}")
}

fn extract_hash<'a>(output: &'a str, prefix: &str) -> &'a str {
    output
        .lines()
        .find_map(|line| {
            let rest = &line[line.find(prefix)? + prefix.len()..];
            rest.split_whitespace().next()
        })
        .expect("could not find hash in output")
}

#[test]
fn test_cache_push_uploads_local_artifact_for_remote_hits() {
    let api = start_mock_api();
    let remote_env = [
        ("TURBO_API", api.as_str()),
        ("TURBO_TOKEN", "token"),
        ("TURBO_TEAM", "team"),
    ];
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    // Build with only the local cache available.
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    assert!(output.status.success(), "{}", combined_output(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = extract_hash(&stdout, "cache miss, executing ").to_string();

    let output = run_turbo_with_env(tempdir.path(), &["cache", "push", &hash], &remote_env);
    assert!(output.status.success(), "{}", combined_output(&output));

    // With the local cache gone, the task can only be restored from the
    // artifact that was pushed.
    fs::remove_dir_all(tempdir.path().join(".turbo/cache")).unwrap();
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--cache=remote:r"],
        &remote_env,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("cache hit, replaying logs {hash}")),
        "{}",
        combined_output(&output)
    );
}

#[test]
fn test_cache_push_fails_for_hash_missing_from_local_cache() {
    let api = start_mock_api();
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["cache", "push", "0123456789abcdef"],
        &[
            ("TURBO_API", api.as_str()),
            ("TURBO_TOKEN", "token"),
            ("TURBO_TEAM", "team"),
        ],
    );
    assert!(!output.status.success());
    let combined = combined_output(&output);
    assert!(
        combined.contains("No local cache artifact found for 0123456789abcdef"),
        "{combined}"
    );
}
