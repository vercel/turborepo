//! `turbo exec` surface: never runs the program without activation, never
//! writes files.
#![cfg_attr(test, allow(clippy::expect_used))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

// Any root configuration enables setup; it needs no future flag.
const ENABLED: &str = "{}";
const REQUIRED: &str = "`turbo exec` requires managed setup";

fn files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![root.to_owned()];
    let mut index = 0;
    while let Some(path) = files.get(index).cloned() {
        if path.is_dir() {
            for entry in fs::read_dir(&path).expect("read fixture directory") {
                files.push(entry.expect("read fixture entry").path());
            }
        }
        index += 1;
    }
    files.sort();
    files
}

/// Only the fixture's `bin` is on PATH; its probe would leave a marker file.
fn invoke(root: &Path, dir: &str, words: &[&str]) -> (i32, String, String) {
    let before = files(root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_turbo"));
    command.env_clear();
    // Keep what Windows needs to find and start a `.cmd` probe.
    #[cfg(windows)]
    for name in ["SystemRoot", "PATHEXT"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = command
        .current_dir(root.join(dir))
        .args(words)
        .env("PATH", root.join("bin"))
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("TURBO_CONFIG_DIR_PATH", root.join("config"))
        .env("VERCEL_CONFIG_DIR_PATH", root.join("config"))
        .env("TURBO_NO_UPDATE_NOTIFIER", "1")
        .env("AI_AGENT", "exec-test")
        .output()
        .expect("run turbo");
    assert_eq!(
        files(root),
        before,
        "{words:?} must not write or run anything"
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Miette may wrap sentences across lines with `|` gutters.
    let stderr = stderr.split([' ', '\n', '|']).collect::<String>();
    let code = output.status.code().expect("turbo exit code");
    (code, stdout, stderr)
}

fn failure(root: &Path, dir: &str, words: &[&str], expected: &str) {
    let (code, stdout, stderr) = invoke(root, dir, words);
    assert_eq!(code, 1, "{words:?}: {stderr}");
    assert_eq!(stdout, "", "{words:?}");
    assert!(
        stderr.contains(&expected.replace(' ', "")),
        "{words:?}: {stderr}"
    );
}

/// A `probe` in the fixture's `bin` that leaves a marker if anything runs it.
fn probe(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let probe = root.join("bin/probe");
        fs::write(&probe, "#!/bin/sh\ntouch \"$(dirname \"$0\")/ran\"\n").expect("probe");
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    #[cfg(windows)]
    fs::write(root.join("bin/probe.cmd"), "@type nul > \"%~dp0ran\"\r\n").expect("probe");
}

#[test]
fn managed_exec_never_reaches_an_unvalidated_local_cli() {
    use turborepo_shim::TurboState;
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let package = root.join("repo/node_modules").join(format!(
        "{}/{}",
        TurboState::scoped_platform_package_scope(),
        TurboState::scoped_platform_package_dir()
    ));
    for dir in [root.join("bin"), package.join("bin")] {
        fs::create_dir_all(dir).expect("fixture directory");
    }
    fs::write(package.join("package.json"), r#"{"version":"2.9.0"}"#).expect("package");
    let pin = package.join("bin").join(TurboState::binary_name());
    // Reaching the pin leaves a marker on Unix and fails to start on Windows.
    fs::write(&pin, "#!/bin/sh\n: > pin-ran\n").expect("pin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pin, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    probe(root);
    let repo = root.join("repo");
    fs::write(
        repo.join("package.json"),
        r#"{"packageManager":"pnpm@10.0.0"}"#,
    )
    .expect("pkg");
    fs::write(repo.join("turbo.json"), ENABLED).expect("turbo.json");
    // A root lock makes the repository managed; setup never prepared it.
    fs::write(repo.join("turbo.lock"), "{}").expect("turbo.lock");
    failure(
        root,
        "repo",
        &["exec", "--", "probe"],
        "managed setup validation failed",
    );
}

#[test]
fn exec_never_runs_the_program_without_activation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    for dir in ["bin", "disabled", "enabled"] {
        fs::create_dir(root.join(dir)).expect("fixture directory");
    }
    probe(root);
    let disabled = root.join("disabled/turbo.json");
    for config in [None, Some("{}")] {
        if let Some(config) = config {
            fs::write(&disabled, config).expect("turbo.json");
        }
        // No future flag is involved: without a lock there is no activation.
        failure(root, "disabled", &["exec", "--", "probe"], REQUIRED);
    }
    fs::write(&disabled, r#"{"futureFlags":{"experimentalSetup":true}}"#).expect("turbo.json");
    failure(
        root,
        "disabled",
        &["exec", "--", "probe"],
        "graduated to stable",
    );
    fs::write(&disabled, "{}").expect("turbo.json");
    fs::write(root.join("enabled/turbo.json"), ENABLED).expect("turbo.json");
    fs::write(root.join("custom.json"), ENABLED).expect("custom.json");

    for words in [
        &["exec", "--", "probe"][..],
        // Words after the separator are the program's, never turbo's.
        &["exec", "--", "probe", "--help"],
        &["exec", "--", "probe", "--version"],
        &[
            "exec",
            "--",
            "probe",
            "--cwd",
            "../disabled",
            "--color=true",
        ],
        &["exec", "--", "probe", "--force", "config"],
    ] {
        failure(root, "enabled", words, REQUIRED);
    }
    // Existing global options select the root.
    for words in [
        &["--cwd", "../enabled", "exec", "--", "probe"][..],
        &["exec", "--root-turbo-json=../custom.json", "--", "probe"],
    ] {
        failure(root, "disabled", words, REQUIRED);
    }
    for (words, expected) in [
        (&["exec", "probe"][..], "can only be given after '--'"),
        (&["exec", "--"], "required arguments were not provided"),
        (
            &["exec", "--", ""],
            "`turbo exec` requires a command after `--`",
        ),
        (
            &["exec", "--update-lock", "--", "probe"],
            "unexpected argument",
        ),
    ] {
        failure(root, "enabled", words, expected);
    }

    // A lock alone is not prepared setup: validation fails before any spawn.
    fs::write(root.join("enabled/turbo.lock"), "{}").expect("turbo.lock");
    failure(
        root,
        "enabled",
        &["exec", "--", "probe"],
        "managed setup validation failed",
    );

    let (code, stdout, _) = invoke(root, "disabled", &["exec", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Usage: turbo exec"), "{stdout}");
}
