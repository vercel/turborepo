#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};

use turborepo_shim::{
    capabilities::Capabilities,
    managed_run::{Selection, select},
};

fn capable_global() -> Capabilities {
    // Synthetic future complete implementation, never the production report.
    Capabilities::parse(
        br#"{"abiVersion":1,"cliVersion":"synthetic-global","managedRunSchemas":[0]}"#,
    )
    .unwrap()
}

fn peer(report: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp
        .path()
        .join(if cfg!(windows) { "peer.exe" } else { "peer" });
    fs::copy(env!("CARGO_BIN_EXE_echo_args"), &binary).unwrap();
    fs::write(temp.path().join("capability-peer.txt"), report).unwrap();
    (temp, binary)
}

#[test]
fn absent_pin_requires_complete_global_schema_membership() {
    let incomplete = Capabilities::unsupported("9999.0.0");
    let error = select(0, &incomplete, None).unwrap_err().to_string();
    assert!(
        error.contains("global CLI") && error.contains("schema 0") && error.contains("9999.0.0")
    );
    assert_eq!(
        select(0, &capable_global(), None).unwrap(),
        Selection::Global
    );
    assert!(select(1, &capable_global(), None).is_err());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn compatible_installed_peer_is_preserved_even_when_global_is_incomplete() {
    let (_temp, binary) =
        peer(r#"{"abiVersion":1,"cliVersion":"old-build-label","managedRunSchemas":[0]}"#);
    assert_eq!(
        select(0, &Capabilities::unsupported("9999.0.0"), Some(&binary)).unwrap(),
        Selection::Local(fs::canonicalize(&binary).unwrap())
    );
    let command = std::process::Command::new(&binary)
        .args(["run", "build", "--", "argument with spaces"])
        .output()
        .unwrap();
    assert!(command.status.success());
    assert_eq!(
        String::from_utf8(command.stdout).unwrap().trim(),
        "run build -- argument with spaces"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn installed_incompatible_pin_is_never_replaced_even_by_a_complete_global() {
    for report in [
        r#"{"abiVersion":1,"cliVersion":"9999.0.0","managedRunSchemas":[]}"#,
        r#"{"abiVersion":1,"cliVersion":"9999.0.0","managedRunSchemas":[1]}"#,
    ] {
        let (_temp, binary) = peer(report);
        let error = select(0, &capable_global(), Some(&binary))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("repository-local CLI")
                && error.contains("schema 0")
                && error.contains("9999.0.0")
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn unsupported_malformed_noisy_nonzero_and_oversized_queries_fail_closed() {
    for report in [
        "not JSON",
        "{}",
        "stderr",
        "nonzero",
        "oversize",
        r#"{"abiVersion":2,"cliVersion":"peer","managedRunSchemas":[0]}"#,
        r#"{"abiVersion":1,"abiVersion":1,"cliVersion":"peer","managedRunSchemas":[0]}"#,
    ] {
        let (_temp, binary) = peer(report);
        let error = select(0, &capable_global(), Some(&binary))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("repository-local CLI") && error.contains("schema 0"),
            "{error}"
        );
    }
    let temp = tempfile::tempdir().unwrap();
    assert!(
        select(
            0,
            &capable_global(),
            Some(&temp.path().join("unresolvable-installed-pin"))
        )
        .is_err()
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn blocked_query_is_bounded_and_its_native_process_is_reaped() {
    let (_temp, binary) = peer("hang");
    let start = Instant::now();
    let error = select(0, &capable_global(), Some(&binary))
        .unwrap_err()
        .to_string();
    assert!(error.contains("timed out"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(10));
    #[cfg(unix)]
    {
        let pid: i32 = fs::read_to_string(binary.parent().unwrap().join("capability-peer.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH)
        );
    }
    // Windows refuses to remove an executable while it is running.
    fs::remove_file(&binary).unwrap();
}

#[test]
fn native_self_case_cannot_claim_schema_readability_as_complete_support() {
    let binary = std::env::current_exe().unwrap();
    assert!(
        select(
            0,
            &Capabilities::unsupported("partial-build"),
            Some(&binary)
        )
        .is_err()
    );
    assert_eq!(
        select(0, &capable_global(), Some(&binary)).unwrap(),
        Selection::Local(fs::canonicalize(binary).unwrap())
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn probe_suppression_does_not_change_the_callers_environment() {
    let (_temp, binary) = peer("environment");
    let before: Vec<_> = std::env::vars_os().collect();
    assert!(matches!(
        select(0, &Capabilities::unsupported("partial"), Some(&binary)).unwrap(),
        Selection::Local(_)
    ));
    assert_eq!(std::env::vars_os().collect::<Vec<_>>(), before);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_gone(pid: i32) {
    let start = Instant::now();
    loop {
        if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
            == Err(nix::errno::Errno::ESRCH)
        {
            return;
        }
        #[cfg(target_os = "linux")]
        if fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
            s.rsplit_once(") ")
                .is_some_and(|(_, fields)| fields.starts_with('Z'))
        }) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "fixture process {pid} survived cleanup"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn exited_parent_does_not_leak_same_group_descendants_with_or_without_pipes() {
    for mode in ["held-pipes", "background"] {
        let (_temp, binary) = peer(mode);
        let start = Instant::now();
        let result = select(0, &capable_global(), Some(&binary));
        if mode == "held-pipes" {
            assert!(result.unwrap_err().to_string().contains("timed out"));
        } else {
            assert!(matches!(result.unwrap(), Selection::Local(_)));
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        let pid = fs::read_to_string(binary.parent().unwrap().join("capability-descendant.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_gone(pid);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn escaped_descendant_cannot_block_the_deadline_or_reader_cleanup() {
    struct FixtureOwner(i32);
    impl Drop for FixtureOwner {
        fn drop(&mut self) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(self.0),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
    let (_temp, binary) = peer("escaped-pipes");
    let start = Instant::now();
    let result = select(0, &capable_global(), Some(&binary));
    let pid = fs::read_to_string(binary.parent().unwrap().join("capability-descendant.pid"))
        .unwrap()
        .parse()
        .unwrap();
    let owner = FixtureOwner(pid);
    assert!(result.unwrap_err().to_string().contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(10));
    // Groups are best-effort cleanup, not confinement of arbitrary native code.
    drop(owner);
    assert_gone(pid);
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn non_self_probe_fails_closed_until_platform_cleanup_is_qualified() {
    let (_temp, binary) = peer("hang");
    let error = select(0, &capable_global(), Some(&binary))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not qualified") && error.contains("schema 0"));
    assert!(
        !binary
            .parent()
            .unwrap()
            .join("capability-peer.pid")
            .exists()
    );
}
