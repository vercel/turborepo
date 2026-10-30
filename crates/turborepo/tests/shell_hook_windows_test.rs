//! Windows PowerShell 5.1 and PowerShell 7 prompts on native Windows, driving
//! `turbo hook powershell` against repositories with real Windows generations
//! and trusted setup records, under a non-ASCII directory that PowerShell
//! would decode with the console code page. Setup refuses Windows arm64
//! hosts, so nothing activates there even with a record present.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use turborepo_tool_install::{Store, Tool};

// A duplicate entry proves the hook removes only what it added, and a
// non-ASCII entry that a console code page cannot corrupt it.
const BASE_PATH: &str = "C:\\Windows\\System32;C:\\Windows;C:\\Windows\\System32\\\
                         WindowsPowerShell\\v1.0;C:\\Windows\\System32;C:\\J\u{fc}rgen\u{2019}s \
                         tools";
const PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// The hook reads the real user state directory, which `LOCALAPPDATA` does
/// not redirect, so records are removed even when an assertion fails.
#[derive(Default)]
struct Records(Vec<PathBuf>);

impl Drop for Records {
    fn drop(&mut self) {
        for record in &self.0 {
            let _ = fs::remove_file(record);
        }
    }
}

/// A clone with its own Git boundary. Repositories given `records` get a real
/// Windows generation (an `.exe` hard link and a `.cmd` forwarder) plus the
/// record `turbo setup` publishes for it, and return its PATH entry.
fn repo(dir: &Path, flagged: bool, records: Option<&mut Records>) -> Option<String> {
    fs::create_dir_all(dir.join(r"apps\web")).unwrap();
    fs::create_dir(dir.join(".git")).unwrap();
    let flags = if flagged {
        r#""futureFlags":{"experimentalSetup":true}"#
    } else {
        ""
    };
    fs::write(dir.join("turbo.json"), format!("{{{flags}}}")).unwrap();
    fs::write(dir.join("turbo.lock"), r#"{"schemaVersion":0,"tools":{}}"#).unwrap();
    let records = records?;
    let root = dir.canonicalize().unwrap();
    let node = Tool {
        id: "node".into(),
        version: "24.0.0".into(),
        platform: "windows-x64".into(),
        artifact_sha256: "a".repeat(64),
        executables: [
            ("node".into(), "node.exe".into()),
            ("npm".into(), "npm.cmd".into()),
        ]
        .into(),
        vars: Default::default(),
    };
    let system32 = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    Store::open(&root)
        .unwrap()
        .reconcile(&[node], |_, tree| {
            // A real PE image stands in for node.exe so running it is observable.
            fs::copy(system32.join("cmd.exe"), tree.join("node.exe"))?;
            fs::write(tree.join("npm.cmd"), "@echo managed-npm %*\r\n")?;
            Ok(())
        })
        .unwrap();
    let record = turborepo_cli::test_support::record_trust_for_tests(&root, None).unwrap();
    records.0.push(record.clone());
    let record: serde_json::Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    let bin = record["installation"]["bin"].as_str().unwrap().to_owned();
    // cmd.exe cannot resolve a forwarder's `%~dp0..` under `\\?\`.
    assert!(!bin.starts_with(r"\\"), "{bin}");
    Some(bin)
}

/// Prints `key=<UTF-8 hex>`, so the console code page cannot change what the
/// test reads.
const SHOW: &str = "function Show($key, $value) { \"$key=\" + (-join \
                    ([Text.Encoding]::UTF8.GetBytes([string]$value) | ForEach-Object { \
                    $_.ToString('x2') })) }";

/// PowerShell commands, each followed by a prompt.
const STEPS: &[&str] = &[
    "Show start $env:PATH",
    "cd ../a/apps/web",
    "Show a $env:PATH",
    "Show node (node /d /c echo managed-node)",
    "Show npm (npm a b)",
    // Without `.CMD`, PowerShell cannot reach the npm forwarder.
    "$env:PATHEXT = '.COM;.EXE;.BAT'",
    "Show pathext $env:PATH",
    "$env:PATHEXT = '.COM;.EXE;.BAT;.CMD'",
    "Show restored $env:PATH",
    "cd ../../vendor/c",
    "Show nested $env:PATH",
    "cd ../../../b",
    "Show b $env:PATH",
    "Add-Content turbo.lock ' '",
    "Show stale $env:PATH",
    "Remove-Item -Recurse -Force .turbo/tools",
    "Show removed $env:PATH",
    "cd ../off",
    "Show off $env:PATH",
    "cd ../plain",
    "Show end \"$env:PATH|$env:__TURBO_HOOK_STATE\"",
];

fn journey(shell: &str) {
    let Ok(program) = which::which(shell) else {
        eprintln!("skipping: {shell} is not installed");
        return;
    };
    let refused = common::windows_arm64_host();
    let dir = tempfile::tempdir().unwrap();
    // Like a profile under a non-ASCII user name. PowerShell itself would
    // close a single-quoted literal at the typographic quote.
    let temp = dir.path().join("J\u{fc}rgen\u{2019}s");
    let mut records = Records::default();
    let a = repo(&temp.join("a"), true, Some(&mut records)).unwrap();
    let b = repo(&temp.join("b"), true, Some(&mut records)).unwrap();
    // Nested, flagged, never set up: untrusted. Flag off: ignored even if set up.
    repo(&temp.join(r"a\vendor\c"), true, None);
    repo(&temp.join("off"), false, Some(&mut records));
    fs::create_dir(temp.join("plain")).unwrap();

    let turbo = assert_cmd::cargo::cargo_bin("turbo");
    let mut script = format!(
        "{SHOW}\nInvoke-Expression (& '{}' hook powershell | Out-String)\n$null = prompt\n",
        turbo.display()
    );
    for step in STEPS {
        // Nothing to run where the hook activates nothing.
        if refused && (step.starts_with("Show node") || step.starts_with("Show npm")) {
            continue;
        }
        // Without a console PowerShell never draws a prompt; call it instead.
        script.push_str(&format!("{step}\n$null = prompt\n"));
    }
    script.push_str("exit\n");
    let mut child = Command::new(program)
        .args(["-NoProfile", "-NoLogo", "-NonInteractive", "-Command", "-"])
        .current_dir(temp.join("plain"))
        .env("PATH", BASE_PATH)
        .env("PATHEXT", PATHEXT)
        .env_remove("__TURBO_HOOK_STATE")
        .env("POWERSHELL_TELEMETRY_OPTOUT", "1")
        .env("POWERSHELL_UPDATECHECK", "Off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let value = |key: &str| {
        let hex = stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("{key} missing\nstdout:\n{stdout}\nstderr:\n{stderr}"));
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        String::from_utf8(bytes).unwrap()
    };
    // PowerShell 7 may add its own directory at startup.
    let start = value("start");
    assert!(start.ends_with(BASE_PATH), "{start}");
    assert_eq!(value("end"), format!("{start}|"));
    for key in ["pathext", "nested", "removed", "off"] {
        assert_eq!(value(key), start, "{key}\n{stderr}");
    }
    if refused {
        for key in ["a", "restored", "b", "stale"] {
            assert_eq!(value(key), start, "{key}\n{stderr}");
        }
        assert!(!stderr.contains("turbo:"), "{stderr}");
        return;
    }
    assert_eq!(value("a"), format!("{a};{start}"), "{stderr}");
    assert_eq!(value("node"), "managed-node");
    assert_eq!(value("npm"), "managed-npm a b");
    assert_eq!(value("restored"), format!("{a};{start}"));
    assert_eq!(value("b"), format!("{b};{start}"));
    assert_eq!(value("stale"), format!("{b};{start}"));
    // Notices appear once on entering or changing state, never every prompt.
    assert_eq!(stderr.matches("turbo:").count(), 4, "{stderr}");
    assert_eq!(
        stderr.matches("need .CMD in PATHEXT").count(),
        1,
        "{stderr}"
    );
    assert_eq!(stderr.matches("not installed").count(), 2, "{stderr}");
    assert_eq!(stderr.matches("turbo.lock changed").count(), 1, "{stderr}");
}

#[test]
fn windows_powershell_hook_activates_only_trusted_set_up_repositories() {
    journey("powershell");
}

#[test]
fn powershell_7_hook_activates_only_trusted_set_up_repositories() {
    journey("pwsh");
}

#[test]
fn posix_shell_hooks_are_unsupported_on_native_windows() {
    let turbo = assert_cmd::cargo::cargo_bin("turbo");
    for shell in ["bash", "zsh", "fish"] {
        let output = Command::new(&turbo).args(["hook", shell]).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(output.stdout.is_empty());
        assert!(
            stderr.contains("not supported on native Windows")
                && stderr.contains("turbo hook powershell"),
            "{stderr}"
        );
        // A hook function installed by an older turbo changes nothing.
        let export = Command::new(&turbo)
            .args(["hook", shell, "--export"])
            .output()
            .unwrap();
        assert_eq!(export.status.code(), Some(0));
        assert!(export.stdout.is_empty() && export.stderr.is_empty());
    }
}
