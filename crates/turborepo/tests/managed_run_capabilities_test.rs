//! Capability discovery is independent of repositories and normal CLI startup.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use std::{
    fs, io,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use turborepo_shim::capabilities::{Capabilities, MAX_RESPONSE_BYTES, QUERY_FLAG};

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(dir: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.push((path.clone(), vec![]));
                visit(&path, files);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    let mut files = vec![];
    visit(root, &mut files);
    files.sort();
    files
}

fn invoke(binary: &Path, root: &Path, args: &[&str]) -> Output {
    let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
    monitor.set_nonblocking(true).unwrap();
    let proxy = format!("http://{}", monitor.local_addr().unwrap());
    let before = snapshot(root);
    let mut command = assert_cmd::Command::new(binary);
    command
        .timeout(Duration::from_secs(10))
        .env_clear()
        .current_dir(root)
        .args(args)
        .env("PATH", "")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", root.join("vercel-config"))
        .env("TURBO_INVOCATION_DIR", root.join("nonexistent-invocation"))
        .env("TURBO_BINARY_PATH", root.join("nonexistent-binary"))
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "1")
        .env("TURBO_NO_UPDATE_NOTIFIER", "0")
        .env("TURBO_TELEMETRY_DISABLED", "0")
        .env("TURBO_API", &proxy)
        .env("TURBO_FORCE", "true")
        .env("__TURBO_WINDOWS_CTRL_C_FD", "invalid")
        .env("AI_AGENT", "capability-test")
        .env("HTTP_PROXY", &proxy)
        .env("HTTPS_PROXY", &proxy)
        .env("ALL_PROXY", &proxy)
        .env("NO_PROXY", "");
    let output = command.output().unwrap();
    assert_eq!(
        snapshot(root),
        before,
        "capability query changed fixture state"
    );
    assert!(
        matches!(monitor.accept(), Err(err) if err.kind() == io::ErrorKind::WouldBlock),
        "capability query contacted HTTP"
    );
    output
}

fn assert_empty_report(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(output.stdout.len() <= MAX_RESPONSE_BYTES);
    let version = include_str!("../../../version.txt").lines().next().unwrap();
    let expected = Capabilities::unsupported(version).encode().unwrap();
    assert_eq!(output.stdout, expected);
    let report = Capabilities::parse(&output.stdout).unwrap();
    assert_eq!(report.cli_version(), version);
    for schema in [0, 1, u32::MAX] {
        assert!(!report.supports_schema(schema));
    }
}

fn binary() -> PathBuf {
    assert_cmd::cargo::cargo_bin("turbo")
}

fn native_package(root: &Path) -> PathBuf {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        os => os,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "64",
        arch => arch,
    };
    let package = root.join(format!("node_modules/@turbo/{os}-{arch}"));
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::write(package.join("package.json"), r#"{"version":"9999.0.0"}"#).unwrap();
    package.join(if cfg!(windows) {
        "bin/turbo.exe"
    } else {
        "bin/turbo"
    })
}

#[test]
fn query_needs_no_js_gate_or_valid_repository_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("AGENTS.md"), "user-owned guidance\n").unwrap();
    fs::write(root.join("turbo.lock"), "invalid prelaunch lock").unwrap();
    fs::write(root.join("package.json"), "invalid package manifest").unwrap();
    fs::write(root.join("Cargo.toml"), "must not probe Cargo").unwrap();
    fs::write(root.join(".npmrc"), "registry=http://127.0.0.1:1\n").unwrap();
    for config in [
        None,
        Some("{}"),
        Some(r#"{"futureFlags":{"experimentalSetup":false}}"#),
        Some(r#"{"futureFlags":{"experimentalSetup":true}}"#),
        Some("invalid root configuration"),
    ] {
        if let Some(config) = config {
            fs::write(root.join("turbo.json"), config).unwrap();
        }
        assert_empty_report(&invoke(&binary(), root, &[QUERY_FLAG]));
    }
    fs::remove_file(root.join("turbo.json")).unwrap();
    fs::write(root.join("turbo.jsonc"), "invalid jsonc").unwrap();
    assert_empty_report(&invoke(&binary(), root, &[QUERY_FLAG]));
}

#[test]
fn global_query_never_hands_off_to_an_installed_pin() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("package.json"), r#"{"name":"fixture","packageManager":"npm@10.5.0","devDependencies":{"turbo":"9999.0.0"}}"#).unwrap();
    fs::write(root.join("turbo.json"), "{}").unwrap();
    let local = native_package(root);
    // Any attempted handoff to this native-package-shaped sentinel must fail.
    fs::write(local, "not an executable; do not run").unwrap();
    assert_empty_report(&invoke(&binary(), root, &[QUERY_FLAG]));
}

#[test]
fn native_local_binary_reports_its_own_empty_capabilities_without_repo_parsing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("turbo.json"), "malformed").unwrap();
    let local = native_package(root);
    fs::copy(binary(), &local).unwrap();
    assert_empty_report(&invoke(&local, root, &[QUERY_FLAG]));
}

#[test]
fn malformed_query_tail_fails_before_profile_and_other_startup_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("turbo.json"), "malformed").unwrap();
    let profile = format!("--profile={}", root.join("profile.json").display());
    let anon = format!("--anon-profile={}", root.join("anon.json").display());
    let heap = format!("--heap={}", root.join("heap.json").display());
    for args in [
        vec![QUERY_FLAG, "--help"],
        vec![QUERY_FLAG, "run", "build"],
        vec![QUERY_FLAG, &profile],
        vec![QUERY_FLAG, &anon],
        vec![QUERY_FLAG, &heap],
        vec![QUERY_FLAG, "--check-for-update"],
        vec![QUERY_FLAG, "--root-turbo-json=missing"],
        vec![QUERY_FLAG, "--"],
        vec!["--__internal-managed-run-capabilities=true"],
    ] {
        let output = invoke(&binary(), root, &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("error: {QUERY_FLAG} must be used alone\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn closed_output_streams_fail_without_panicking_or_writing_crash_reports() {
    use std::{
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::Stdio,
    };

    let temp = tempfile::tempdir().unwrap();
    for args in [vec![QUERY_FLAG], vec![QUERY_FLAG, "--help"]] {
        let closed_stream = || {
            let (reader, writer) = UnixStream::pair().unwrap();
            drop(reader);
            Stdio::from(OwnedFd::from(writer))
        };
        let before = snapshot(temp.path());
        let mut child = std::process::Command::new(binary())
            .args(args)
            .env_clear()
            .current_dir(temp.path())
            .env("PATH", "")
            .env("HOME", temp.path().join("home"))
            .env("TURBO_CONFIG_DIR_PATH", temp.path().join("config"))
            .stdin(Stdio::null())
            .stdout(closed_stream())
            .stderr(closed_stream())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < Duration::from_secs(10) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("query failed to exit: {result:?}");
                }
            }
        };
        assert_eq!(
            status.code(),
            Some(1),
            "must return an IO/usage error, not panic"
        );
        assert_eq!(snapshot(temp.path()), before);
    }
}

#[test]
fn internal_query_flag_is_not_exposed_in_public_help() {
    let temp = tempfile::tempdir().unwrap();
    let output = assert_cmd::Command::new(binary())
        .arg("--help")
        .current_dir(temp.path())
        .env("DO_NOT_TRACK", "1")
        .env("TURBO_NO_UPDATE_NOTIFIER", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(QUERY_FLAG));
}
