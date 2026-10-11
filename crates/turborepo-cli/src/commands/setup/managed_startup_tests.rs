//! Actual native CLI consumes records published by the existing setup fixture.
//! This qualifies startup wiring, not vendor Node/pnpm binaries or the full
//! ABI. Run after building the current stack's `turbo` with
//! TURBO_TEST_BINARY_PATH.
use std::{
    io::{Read, Seek, SeekFrom},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

use turborepo_shim::{
    TurboState,
    capabilities::{Capabilities, QUERY_FLAG},
};

use super::*;

fn binary() -> PathBuf {
    PathBuf::from(
        std::env::var_os("TURBO_TEST_BINARY_PATH")
            .unwrap_or_else(|| panic!("build this stack's turbo and set TURBO_TEST_BINARY_PATH")),
    )
    .canonicalize()
    .unwrap()
}

fn invoke(f: &Fixture, executable: &std::path::Path, args: &[&str]) -> Output {
    let mut stdout = tempfile::tempfile().unwrap();
    let mut stderr = tempfile::tempfile().unwrap();
    let mut child = Command::new(executable)
        .env_clear()
        .current_dir(f.owned.root())
        .args(args)
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", f.owned.home())
        .env("XDG_DATA_HOME", f.owned.home().join("data"))
        .env("XDG_CONFIG_HOME", f.owned.home().join("config"))
        .env("TURBO_CONFIG_DIR_PATH", f.owned.home().join("config"))
        .env("DO_NOT_TRACK", "1")
        .env("TURBO_NO_UPDATE_NOTIFIER", "1")
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "1")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .stdin(Stdio::null())
        .stdout(stdout.try_clone().unwrap())
        .stderr(stderr.try_clone().unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native startup timed out: {args:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    stdout.seek(SeekFrom::Start(0)).unwrap();
    stderr.seek(SeekFrom::Start(0)).unwrap();
    let mut out = Output {
        status,
        stdout: vec![],
        stderr: vec![],
    };
    stdout.read_to_end(&mut out.stdout).unwrap();
    stderr.read_to_end(&mut out.stderr).unwrap();
    out
}

fn prepared(f: &Fixture, upstream: &Upstream) {
    f.run(upstream, FROZEN).unwrap();
    // Relocate the producer's unchanged private evidence into this child's
    // normal user-data directory. No forged records or runtime bypass flags.
    let data = if cfg!(target_os = "macos") {
        f.owned.home().join("Library/Application Support")
    } else {
        f.owned.home().join("data")
    };
    fs::create_dir_all(&data).unwrap();
    fs::rename(f.owned.home().join("turborepo"), data.join("turborepo")).unwrap();
}

fn installed(f: &Fixture, copy: bool) -> PathBuf {
    let package = f
        .owned
        .root()
        .join("node_modules")
        .join(TurboState::scoped_platform_package_scope())
        .join(TurboState::scoped_platform_package_dir());
    let executable = package.join("bin").join(TurboState::binary_name());
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(
        package.join("package.json"),
        json!({"version": crate::get_version()}).to_string(),
    )
    .unwrap();
    if copy {
        fs::copy(binary(), &executable).unwrap();
    } else {
        std::os::unix::fs::symlink(binary(), &executable).unwrap();
    }
    executable
}

#[test]
#[ignore = "requires current native turbo at TURBO_TEST_BINARY_PATH"]
fn actual_startup_uses_valid_evidence_and_tests_both_real_local_capability_paths() {
    for case in [None, Some(false), Some(true)] {
        let f = super::managed_run::runnable();
        let u = Upstream::new(&f, false, None);
        prepared(&f, &u);
        let executable = case.map(|copy| installed(&f, copy));
        let entry = if case == Some(false) {
            executable.as_ref().unwrap().clone()
        } else {
            binary()
        };
        let query = invoke(&f, &entry, &[QUERY_FLAG]);
        assert!(query.status.success(), "{query:?}");
        let report = Capabilities::parse(&query.stdout).unwrap();
        assert_eq!(report, Capabilities::current(crate::get_version()));
        if let Some(local) = &executable {
            let query = invoke(&f, local, &[QUERY_FLAG]);
            assert!(query.status.success(), "{query:?}");
            assert_eq!(Capabilities::parse(&query.stdout).unwrap(), report);
        }
        for mode in ["strict", "loose"] {
            let args = [
                "run",
                "build",
                if mode == "strict" {
                    "--env-mode=strict"
                } else {
                    "--env-mode=loose"
                },
                "--ui=stream",
                "--cache=local:rw",
            ];
            let output = invoke(&f, &entry, &args);
            let result = f.owned.root().join("apps/web/managed-result");
            if case.is_none() || report.supports_schema(0) {
                assert!(output.status.success(), "case={case:?} {mode}: {output:?}");
                let result_bytes = fs::read_to_string(&result).unwrap();
                let bin = turborepo_tool_install::Store::inspect(f.owned.root())
                    .unwrap()
                    .unwrap()
                    .bin;
                let lines: Vec<_> = result_bytes.lines().collect();
                assert_eq!(
                    std::path::Path::new(lines[0]).canonicalize().unwrap(),
                    bin.join("node").canonicalize().unwrap()
                );
                assert_eq!(
                    std::env::split_paths(std::ffi::OsStr::new(lines[1]))
                        .next()
                        .unwrap(),
                    bin
                );
                assert!(!lines[2].is_empty());
                fs::remove_file(result).unwrap();
            } else {
                // This is an honest unqualified-ABI assertion, not a positive
                // local-run proof. Promotion must additionally pass qualification.
                assert_eq!(output.status.code(), Some(1), "{output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("does not advertise"),
                    "{output:?}"
                );
                assert!(!result.exists());
            }
            assert_eq!(u.hits(), 3, "run must not fetch or repair implicitly");
        }
    }
}

#[test]
#[ignore = "release gate: requires qualified initial Unix contract and current native binary"]
fn qualified_current_cli_must_run_both_self_local_and_identical_installed_copy() {
    let f = super::managed_run::runnable();
    let query = invoke(&f, &binary(), &[QUERY_FLAG]);
    let report = Capabilities::parse(&query.stdout).unwrap();
    assert!(
        report.supports_schema(0),
        "schema 0 is not qualified; do not advertise support to make this test pass"
    );
    actual_startup_uses_valid_evidence_and_tests_both_real_local_capability_paths();
}
