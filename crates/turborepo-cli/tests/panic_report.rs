#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! Exercise the installed panic hook in a subprocess so its output and CI
//! environment do not interfere with other tests.

use std::{
    path::PathBuf,
    process::{Command, Output},
};

const CHILD_ENV: &str = "TURBO_PANIC_REPORT_TEST_CHILD";
const PANIC_MESSAGE: &str = "panic report regression test";

#[test]
fn panic_report_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }

    std::panic::set_hook(Box::new(turborepo_cli::panic_handler));
    assert!(std::panic::catch_unwind(|| panic!("{PANIC_MESSAGE}")).is_err());
}

fn run_child(ci: bool, temp_dir_is_file: bool) -> (Output, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let tmpdir = if temp_dir_is_file {
        let path = temp_dir.path().join("not-a-directory");
        std::fs::write(&path, "").unwrap();
        path
    } else {
        temp_dir.path().to_path_buf()
    };
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "panic_report_child", "--nocapture"])
        .env_clear()
        .env(CHILD_ENV, "1")
        .env("TMPDIR", tmpdir);
    if ci {
        command.env("CI", "true");
    }

    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (output, temp_dir)
}

fn assert_report(report: &str) {
    let report: toml::Value = toml::from_str(report).expect("crash report must be valid TOML");
    assert_eq!(report["name"].as_str(), Some("turbo"));
    assert_eq!(report["method"].as_str(), Some("Panic"));
    assert!(report["operating_system"]
        .as_str()
        .is_some_and(|os| !os.is_empty()));
    assert!(report["crate_version"]
        .as_str()
        .is_some_and(|version| !version.is_empty()));
    assert!(report["explanation"]
        .as_str()
        .is_some_and(|text| text.contains("panic_report.rs")));
    assert!(report["cause"]
        .as_str()
        .is_some_and(|cause| cause.contains(PANIC_MESSAGE)));
    assert!(report["backtrace"]
        .as_str()
        .is_some_and(|trace| !trace.is_empty()));
}

#[test]
fn saves_a_crash_report_outside_ci() {
    let (output, temp_dir) = run_child(false, false);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Oops! Turbo has crashed."));
    assert!(stderr
        .contains("Please open an issue at https://github.com/vercel/turborepo/issues/new/choose"));

    let path = stderr
        .lines()
        .find_map(|line| line.strip_prefix("A report has been written to "))
        .map(PathBuf::from)
        .expect("panic handler should report the saved file path");
    assert!(path.starts_with(temp_dir.path()), "{path:?}");
    assert_eq!(path.extension().unwrap(), "toml");
    let report = std::fs::read_to_string(path).unwrap();
    assert_report(&report);
}

#[test]
fn reports_a_failure_to_save_without_panicking() {
    let (output, _) = run_child(false, true);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Oops! Turbo has crashed."));
    assert!(stderr.contains("An error has occurred while attempting to write a report."));
    assert!(stderr
        .contains("Please open an issue at https://github.com/vercel/turborepo/issues/new/choose"));
}

#[test]
fn prints_a_crash_report_in_ci_without_saving_it() {
    let (output, temp_dir) = run_child(true, false);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Oops! Turbo has crashed."));
    assert!(stderr.contains("Caused by \n"));
    let report = stderr
        .split_once("Caused by \n")
        .unwrap()
        .1
        .split_once("\n\nPlease open an issue")
        .unwrap()
        .0;
    assert_report(report);
    assert!(!stderr.contains("A report has been written to"));
    assert_eq!(temp_dir.path().read_dir().unwrap().count(), 0);
}
