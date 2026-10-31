//! Interrupts `turbo setup` through the standalone binary and the installed
//! npm wrapper while the managed Node download is blocked. Setup starts no
//! installer subprocess today, so a loopback proxy that accepts the tunnel and
//! never answers holds the download open without contacting nodejs.org.
#![cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader},
    net::TcpListener,
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use common::{
    combined_output,
    process::unix::{TaskTreeGuard, wait_for_process_gone},
    process_launcher::{CapturedProcess, Entrypoint, ProcessLauncher},
};
use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};

const TIMEOUT: Duration = Duration::from_secs(20);

/// Accepts proxy tunnels, reports each request line, and never answers.
fn stalled_proxy() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            held.push(stream);
            if tx.send(line.trim_end().to_owned()).is_err() {
                return;
            }
        }
    });
    (url, rx)
}

/// A Node-only managed repository with a committed lock for this host.
fn fixture(root: &Path) {
    let (selector, spelling) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("macos-arm64", "darwin-arm64"),
        ("macos", _) => ("macos-x64", "darwin-x64"),
        ("linux", "aarch64") => ("linux-arm64-gnu", "linux-arm64"),
        _ => ("linux-x64-gnu", "linux-x64"),
    };
    let prefix = format!("node-v24.0.0-{spelling}");
    let lock = serde_json::json!({"schemaVersion": 0, "tools": {"node": {
        "adapter": "node", "version": "24.0.0",
        "declarations": [{"file": ".nvmrc", "request": "24.0.0"}],
        "installation": {"kind": "managed", "artifacts": {selector: {"distribution": {
            "url": format!("https://nodejs.org/dist/v24.0.0/{prefix}.tar.gz"),
            "sha256": "0".repeat(64), "format": "tar-gz", "rootPrefix": prefix,
            "executables": {"node": "bin/node"}
        }}}}
    }}});
    fs::create_dir_all(root).unwrap();
    for (name, contents) in [
        (
            "turbo.json",
            r#"{"futureFlags":{"experimentalSetup":true}}"#.to_owned(),
        ),
        (
            "package.json",
            r#"{"name":"fixture","private":true}"#.to_owned(),
        ),
        (".nvmrc", "24.0.0\n".to_owned()),
        (".gitignore", "/.turbo/\n".to_owned()),
        // Canonical bytes, so local setup has no reason to rewrite the lock.
        ("turbo.lock", format!("{lock}\n")),
    ] {
        fs::write(root.join(name), contents).unwrap();
    }
    common::git(root, &["init", "--quiet"]);
}

/// Repository files with contents, plus the shape of ignored setup state.
fn state(root: &Path) -> (BTreeMap<String, Vec<u8>>, Vec<String>) {
    fn visit(
        root: &Path,
        dir: &Path,
        files: &mut BTreeMap<String, Vec<u8>>,
        setup: &mut Vec<String>,
    ) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if relative == ".git" {
                continue;
            }
            if relative.starts_with(".turbo") {
                setup.push(format!(
                    "{relative}{}",
                    if path.is_dir() { "/" } else { "" }
                ));
            }
            if path.is_dir() {
                visit(root, &path, files, setup);
            } else if !relative.starts_with(".turbo") {
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    let (mut files, mut setup) = (BTreeMap::new(), Vec::new());
    visit(root, root, &mut files, &mut setup);
    setup.sort();
    (files, setup)
}

fn command(launcher: &ProcessLauncher, entry: Entrypoint, root: &Path, proxy: &str) -> Command {
    let mut command = launcher.command(entry, root);
    // Isolate child-process configuration like the standalone setup tests.
    let home = root.parent().unwrap().join("home");
    command
        .env_clear()
        .args(["setup", "--tools-only"])
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("TURBO_CONFIG_DIR_PATH", &home)
        .env("VERCEL_CONFIG_DIR_PATH", &home)
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .env("HTTP_PROXY", proxy)
        .env("HTTPS_PROXY", proxy)
        .env("ALL_PROXY", proxy)
        .env("NO_PROXY", "");
    command
}

/// Starts setup and returns once it is blocked on the Node download.
fn start_blocked(
    launcher: &ProcessLauncher,
    entry: Entrypoint,
    root: &Path,
) -> (CapturedProcess, Vec<i32>, Vec<TaskTreeGuard>) {
    let (proxy, requests) = stalled_proxy();
    let mut process = CapturedProcess::spawn(&mut command(launcher, entry, root, &proxy));
    let start = Instant::now();
    let request = loop {
        if let Ok(request) = requests.recv_timeout(Duration::from_millis(50)) {
            break request;
        }
        if process.child.try_wait().unwrap().is_some() {
            let output = process.finish(TIMEOUT);
            panic!(
                "setup exited before downloading\n{}",
                combined_output(&output)
            );
        }
        assert!(
            start.elapsed() < TIMEOUT,
            "setup never reached the download"
        );
    };
    assert_eq!(request, "CONNECT nodejs.org:443 HTTP/1.1");
    // The wrapper's native child, plus anything native setup started.
    let mut pids = Vec::new();
    let mut parents = vec![process.child.id() as i32];
    while let Some(parent) = parents.pop() {
        let output = Command::new("pgrep")
            .arg("-P")
            .arg(parent.to_string())
            .output()
            .unwrap();
        for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
            pids.push(pid.parse().unwrap());
            parents.push(pid.parse().unwrap());
        }
    }
    let guards = pids.iter().map(|pid| TaskTreeGuard::new(*pid)).collect();
    (process, pids, guards)
}

fn stop(process: &CapturedProcess, request: Signal) {
    let pid = process.child.id() as i32;
    match request {
        // Ctrl+C reaches the whole foreground group, wrapper and native alike.
        Signal::SIGINT => signal::kill(Pid::from_raw(-pid), Signal::SIGINT).unwrap(),
        // A process manager signals only the entrypoint PID.
        _ => signal::kill(Pid::from_raw(pid), request).unwrap(),
    }
}

#[test]
fn interrupted_download_releases_locks_and_leaves_resumable_state() {
    let launcher = ProcessLauncher::new();
    for request in [Signal::SIGINT, Signal::SIGTERM] {
        let mut results = Vec::new();
        for entry in [Entrypoint::Standalone, Entrypoint::NpmWrapper] {
            eprintln!("case: {entry:?} {request:?}");
            let temp = tempfile::tempdir().unwrap();
            let root: PathBuf = temp.path().canonicalize().unwrap().join("repo");
            fixture(&root);
            let (before, _) = state(&root);
            let (process, pids, mut guards) = start_blocked(&launcher, entry, &root);
            let entrypoint = process.child.id() as i32;
            stop(&process, request);
            let output = process.finish(TIMEOUT);
            let context = format!("{entry:?} {request:?}\n{}", combined_output(&output));
            for pid in std::iter::once(entrypoint).chain(pids) {
                wait_for_process_gone(pid, TIMEOUT);
            }
            guards.iter_mut().for_each(TaskTreeGuard::disarm);
            assert_eq!(output.status.signal(), Some(request as i32), "{context}");
            let (after, setup) = state(&root);
            assert_eq!(
                after, before,
                "committed files, including turbo.lock\n{context}"
            );
            // Only the reusable writer lock remains in the repository, and the
            // store holds no published tree, environment or record.
            assert_eq!(
                setup,
                [".turbo/", ".turbo/setup-lock/", ".turbo/setup-lock/writer"],
                "{context}"
            );
            let store = common::setup::user_tool_store(&root.parent().unwrap().join("home"), None);
            for dir in ["trees", "envs"] {
                let entries = fs::read_dir(store.join(dir)).map_or(0, |d| d.count());
                assert_eq!(entries, 0, "{dir}\n{context}");
            }
            assert!(
                common::setup::installed(&root, &store).is_none(),
                "{context}"
            );

            // A retry reaches the download again instead of waiting on a stale
            // writer lock or rejecting the interrupted state.
            let (retry, pids, mut guards) = start_blocked(&launcher, entry, &root);
            let retry_pid = retry.child.id() as i32;
            stop(&retry, Signal::SIGTERM);
            retry.finish(TIMEOUT);
            for pid in std::iter::once(retry_pid).chain(pids) {
                wait_for_process_gone(pid, TIMEOUT);
            }
            guards.iter_mut().for_each(TaskTreeGuard::disarm);
            assert_eq!(state(&root).0, before, "{context}");
            results.push((output.status.signal(), output.stdout, output.stderr, setup));
        }
        assert_eq!(
            results[0], results[1],
            "entrypoints disagree for {request:?}"
        );
    }
}
