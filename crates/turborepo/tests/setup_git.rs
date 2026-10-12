//! Setup explains missing Git and non-repository roots before any writer.
#![cfg(all(
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))
))]
#![allow(clippy::unwrap_used)]

use std::{
    fs, io,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
};

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = vec![];
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(snapshot(&path));
            files.push((path, vec![]));
        } else {
            let bytes = fs::read(&path).unwrap();
            files.push((path, bytes));
        }
    }
    files.sort();
    files
}

/// Runs local first-lock setup and returns stderr without wrapping or gutters.
fn failed_setup(root: &Path, path: &Path) -> String {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let url = format!("http://{}", proxy.local_addr().unwrap());
    let before = snapshot(root);
    let output = Command::new(env!("CARGO_BIN_EXE_turbo"))
        .env_clear()
        .current_dir(root)
        .args(["setup", "--tools-only", "--no-frozen"])
        .env("PATH", path)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", root.join("config"))
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("HTTP_PROXY", &url)
        .env("HTTPS_PROXY", &url)
        .env("ALL_PROXY", &url)
        .env("NO_PROXY", "")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(snapshot(root), before, "setup must not write");
    assert!(
        matches!(proxy.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
        "unexpected HTTP connection"
    );
    stderr.replace([' ', '|', '\n'], "")
}

#[test]
fn missing_git_and_non_repository_roots_get_actionable_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(
        root.join("turbo.json"),
        r#"{"futureFlags":{"experimentalSetup":true}}"#,
    )
    .unwrap();
    fs::write(root.join(".nvmrc"), "24.x").unwrap();

    let text = failed_setup(root, Path::new(""));
    assert!(
        text.contains("couldnotrun`git`:itwasnotfoundonPATH"),
        "{text}"
    );
    assert!(!text.contains("(oserror2)"), "{text}");

    // With Git available, a non-repository root is a Git failure, not a
    // missing ignore rule that editing .gitignore could fix.
    let bin = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(which::which("git").unwrap(), bin.path().join("git")).unwrap();
    let text = failed_setup(root, bin.path());
    assert!(
        text.contains("Gitcouldnotcheckwhether.turbo/isignored"),
        "{text}"
    );
    assert!(!text.contains(".gitignore"), "{text}");
}
