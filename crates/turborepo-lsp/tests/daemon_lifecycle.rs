#![allow(clippy::unwrap_used)]

use std::{path::Path, process::Command};

fn run_daemon(repo_root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_turborepo-lsp"))
        .current_dir(repo_root)
        .arg("daemon")
        .args(args)
        .output()
        .unwrap()
}

struct StopOnDrop<'a>(&'a Path);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let _ = run_daemon(self.0, &["stop"]);
    }
}

#[test]
fn manages_daemon_lifecycle() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(
        repo.path().join("package.json"),
        r#"{"name":"daemon-lifecycle-test","packageManager":"pnpm@10.28.0"}"#,
    )
    .unwrap();
    let _stop_on_drop = StopOnDrop(repo.path());

    let start = run_daemon(repo.path(), &["start"]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&start.stdout),
        "✓ daemon is running\n"
    );

    let status = run_daemon(repo.path(), &["status", "--json"]);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(status["uptime_ms"].is_u64());
    assert!(status["pid_file"].is_string());
    assert!(status["sock_file"].is_string());
    assert!(Path::new(status["log_file"].as_str().unwrap()).exists());

    let restart = run_daemon(repo.path(), &["restart"]);
    assert!(
        restart.status.success(),
        "{}",
        String::from_utf8_lossy(&restart.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&restart.stdout),
        "✓ restarted daemon\n"
    );

    let stop = run_daemon(repo.path(), &["stop"]);
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&stop.stdout), "✓ stopped daemon\n");

    let clean = run_daemon(repo.path(), &["clean"]);
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&clean.stdout), "Done\n");
}
