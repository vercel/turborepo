//! Native entrypoint CI diagnostics fail before writes, probes or HTTP.
#![cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#![allow(clippy::unwrap_used)]

use std::{fs, io, net::TcpListener, path::Path, process::Command};

fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut files = vec![];
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            files.extend(snapshot(&path));
        }
        let bytes = if metadata.is_file() {
            fs::read(&path).unwrap()
        } else {
            vec![]
        };
        files.push((path, bytes, metadata.modified().unwrap()));
    }
    files.sort();
    files
}

#[test]
fn standalone_ci_missing_and_stale_locks_render_local_repair_guidance_without_writes() {
    for (lock, declaration) in [(false, "24.0.0"), (true, "24.x")] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("turbo.json"), "{}").unwrap();
        fs::write(root.join(".nvmrc"), declaration).unwrap();
        if lock {
            // The stale declaration must be diagnosed before the executor
            // considers this unsupported installation kind.
            fs::write(root.join("turbo.lock"), r#"{"schemaVersion":0,"tools":{"node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],"installation":{"kind":"verify-system","executables":["node"]}}}}"#).unwrap();
        }
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let url = format!("http://{}", proxy.local_addr().unwrap());
        let before = snapshot(root);
        let modified = fs::metadata(root).unwrap().modified().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
            .env_clear()
            .current_dir(root)
            .args(["setup", "--tools-only"])
            .env("CI", "1")
            .env("PATH", "")
            .env("HOME", root.join("home"))
            .env("TURBO_TELEMETRY_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("HTTP_PROXY", &url)
            .env("HTTPS_PROXY", &url)
            .env("ALL_PROXY", &url)
            .env("NO_PROXY", "")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .replace([' ', '|', '\n', '\r'], "");
        assert!(
            message.contains(if lock {
                "turbo.lockdeclarationschanged:node:.nvmrc"
            } else {
                "frozenmoderequiresturbo.lock"
            }),
            "{message}"
        );
        assert!(message.contains("turbosetup--tools-only"), "{message}");
        assert!(
            message.contains("locally") && message.contains("commit"),
            "{message}"
        );
        assert_eq!(snapshot(root), before);
        assert_eq!(fs::metadata(root).unwrap().modified().unwrap(), modified);
        assert!(matches!(proxy.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock));
    }
}
