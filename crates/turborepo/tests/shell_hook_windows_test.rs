//! Windows PowerShell 5.1 and PowerShell 7 prompts on native Windows. Without
//! this user's trusted setup record the hook activates nothing and keeps PATH
//! exactly as the shell started; on x64 it reports the missing installation.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

// A duplicate entry proves nothing rewrites PATH.
const BASE_PATH: &str = r"C:\Windows\System32;C:\Windows;C:\Windows\System32\WindowsPowerShell\v1.0;C:\Windows\System32";

fn journey(shell: &str) {
    let Ok(program) = which::which(shell) else {
        eprintln!("skipping: {shell} is not installed");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    for (name, flags) in [
        ("on", r#"{"futureFlags":{"experimentalSetup":true}}"#),
        ("off", "{}"),
    ] {
        let repo = temp.path().join(name);
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join("turbo.json"), flags).unwrap();
        fs::write(repo.join("turbo.lock"), r#"{"schemaVersion":0,"tools":{}}"#).unwrap();
    }
    fs::create_dir(temp.path().join("plain")).unwrap();

    let turbo = assert_cmd::cargo::cargo_bin("turbo");
    let mut script = format!(
        "Invoke-Expression (& '{}' hook powershell | Out-String)\n",
        turbo.display()
    );
    for line in [
        "\"start=$env:PATH\"",
        "cd ../on",
        "\"on=$env:PATH|$env:__TURBO_HOOK_STATE\"",
        "cd ../off",
        "\"off=$env:PATH\"",
        "cd ../plain",
        "\"end=$env:PATH|$env:__TURBO_HOOK_STATE\"",
    ] {
        // Without a console PowerShell never draws a prompt; call it instead.
        script.push_str(&format!("$null = prompt\n{line}\n"));
    }
    script.push_str("exit\n");
    let mut child = Command::new(program)
        .args(["-NoProfile", "-NoLogo", "-NonInteractive", "-Command", "-"])
        .current_dir(temp.path().join("plain"))
        .env("PATH", BASE_PATH)
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
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("{key} missing\nstdout:\n{stdout}\nstderr:\n{stderr}"))
            .to_owned()
    };
    // PowerShell 7 may add its own directory at startup.
    let start = value("start");
    assert!(start.ends_with(BASE_PATH), "{start}");
    let on = value("on");
    let (path, state) = on.split_once('|').unwrap();
    assert_eq!(path, start);
    // The hook ran; setup records trusted roots only on Windows x64 (#14659).
    let x64 = cfg!(target_arch = "x86_64");
    let status = if x64 { ";missing;" } else { ";off;" };
    assert!(state.starts_with("1;") && state.contains(status), "{on}");
    assert_eq!(value("off"), start);
    assert_eq!(value("end"), format!("{start}|"));
    // One "not installed" (or unreadable user state) notice on entering.
    let notices = stderr.matches("this repository's managed tools").count();
    assert_eq!(notices, usize::from(x64), "{stderr}");
}

#[test]
fn windows_powershell_hook_keeps_path_unchanged() {
    journey("powershell");
}

#[test]
fn powershell_7_hook_keeps_path_unchanged() {
    journey("pwsh");
}
