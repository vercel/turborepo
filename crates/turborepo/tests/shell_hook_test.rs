//! Real bash, zsh, fish, and PowerShell sessions driving `turbo hook` at each
//! prompt.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use sha2::{Digest, Sha256};
use turborepo_tool_install::{Store, Tool};

// A duplicate entry proves the hook removes only what it added.
const BASE_PATH: &str = "/usr/bin:/bin:/usr/bin";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn user_state(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        home.join("data")
    }
}

/// A clone with its own Git boundary. `home` set-up repositories get a real
/// installation plus the external record `turbo setup` publishes for it.
fn repo(dir: &Path, flagged: bool, home: Option<&Path>) -> Option<String> {
    fs::create_dir_all(dir.join("apps/web")).unwrap();
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir(dir.join(".git")).unwrap();
    let flags = if flagged {
        r#""futureFlags":{"experimentalSetup":true}"#
    } else {
        ""
    };
    fs::write(dir.join("turbo.json"), format!("{{{flags}}}")).unwrap();
    fs::write(dir.join("turbo.lock"), r#"{"schemaVersion":0,"tools":{}}"#).unwrap();
    let home = home?;
    let root = dir.canonicalize().unwrap();
    let tool = Tool {
        id: "node".into(),
        version: "24.0.0".into(),
        platform: "test".into(),
        artifact_sha256: "a".repeat(64),
        executables: [("node".into(), "bin/node".into())].into(),
    };
    Store::open(&root)
        .unwrap()
        .reconcile(&[tool], |_, tree| {
            fs::create_dir_all(tree.join("bin"))?;
            fs::write(tree.join("bin/node"), "#!/bin/sh\necho managed-node\n")?;
            fs::set_permissions(tree.join("bin/node"), fs::Permissions::from_mode(0o755))?;
            Ok(())
        })
        .unwrap();
    let current = Store::inspect(&root).unwrap().unwrap();
    let identity = fs::metadata(&root).unwrap();
    let record = serde_json::json!({
        "schema": 1,
        "lock_sha256": sha(&fs::read(root.join("turbo.lock")).unwrap()),
        "installation": {
            "root": root,
            "device": identity.dev(),
            "inode": identity.ino(),
            "manifest_sha256": sha(&fs::read(root.join(".turbo/tools/manifest.json")).unwrap()),
            "bin": current.bin,
            "tools": current.tools,
        },
    });
    let records = user_state(home).join("turborepo/setup-roots");
    fs::create_dir_all(&records).unwrap();
    for dir in records.ancestors().take_while(|dir| dir.starts_with(home)) {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let path = records.join(format!("{}.json", sha(root.as_os_str().as_bytes())));
    fs::write(&path, record.to_string()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    Some(current.bin.to_str().unwrap().to_owned())
}

/// One shell-neutral command per prompt; each shell renders its own syntax.
enum Step {
    Cd(&'static str),
    Show(&'static str),
    Node,
    TouchLock,
    RemoveTools,
    Nothing,
}

const STEPS: &[Step] = &[
    Step::Show("start"),
    Step::Cd("../a/apps/web"),
    Step::Show("a"),
    Step::Node,
    Step::Cd("../../vendor/c"),
    Step::Show("nested"),
    Step::Cd("../../../b"),
    Step::Show("b"),
    Step::TouchLock,
    Step::Show("stale"),
    Step::Nothing,
    Step::RemoveTools,
    Step::Show("removed"),
    Step::Cd("../off"),
    Step::Show("off"),
    Step::Cd("../plain"),
    Step::Show("end"),
];

fn line(shell: &str, step: &Step) -> String {
    let pwsh = shell == "pwsh";
    match step {
        Step::Cd(dir) => format!("cd {dir}"),
        Step::Show(key) if pwsh => format!("\"{key}=$env:PATH\""),
        Step::Show(key) => format!("echo \"{key}=$PATH\""),
        Step::Node if pwsh => "\"node=$(node)\"".into(),
        Step::Node if shell == "fish" => "echo \"node=\"(node)".into(),
        Step::Node => "echo \"node=$(node)\"".into(),
        Step::TouchLock if pwsh => "Add-Content turbo.lock ' '".into(),
        Step::TouchLock => "printf ' ' >> turbo.lock".into(),
        Step::RemoveTools if pwsh => "Remove-Item -Recurse -Force .turbo/tools".into(),
        Step::RemoveTools => "rm -rf .turbo/tools".into(),
        Step::Nothing if pwsh => "$null = 0".into(),
        Step::Nothing => "true".into(),
    }
}

fn journey(shell: &str) {
    let Ok(program) = which::which(shell) else {
        eprintln!("skipping: {shell} is not installed");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let a = repo(&temp.path().join("a"), true, Some(&home)).unwrap();
    let b = repo(&temp.path().join("b"), true, Some(&home)).unwrap();
    // Nested, flagged, never set up: untrusted. Flag off: ignored even if set up.
    repo(&temp.path().join("a/vendor/c"), true, None);
    repo(&temp.path().join("off"), false, Some(&home));
    fs::create_dir(temp.path().join("plain")).unwrap();

    let turbo = assert_cmd::cargo::cargo_bin("turbo");
    let turbo = turbo.display();
    // bash and zsh run real prompts. Without a terminal, fish and pwsh never
    // draw one, so the test fires their prompt hooks after every command.
    let (args, install, prompt): (&[&str], _, _) = match shell {
        "bash" => (
            &["--norc", "--noprofile", "-i"],
            format!("eval \"$('{turbo}' hook bash)\""),
            "",
        ),
        "zsh" => (&["-f", "-i"], format!("eval \"$('{turbo}' hook zsh)\""), ""),
        "fish" => (
            &["--no-config"],
            format!("'{turbo}' hook fish | source"),
            "\nemit fish_prompt",
        ),
        _ => (
            &["-NoProfile", "-NoLogo", "-NonInteractive", "-Command", "-"],
            format!("Invoke-Expression (& '{turbo}' hook powershell | Out-String)"),
            "\n$null = prompt",
        ),
    };
    let mut script = format!("{install}{prompt}\n");
    for step in STEPS {
        script.push_str(&format!("{}{prompt}\n", line(shell, step)));
    }
    script.push_str("exit\n");
    let mut child = Command::new(program)
        .args(args)
        .current_dir(temp.path().join("plain"))
        .env_clear()
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("PATH", BASE_PATH)
        .env("TERM", "dumb")
        .env("PS1", "")
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let value = |key: &str| {
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("{key} missing\nstdout:\n{stdout}\nstderr:\n{stderr}"))
            .to_owned()
    };
    // pwsh may add its own directory at startup; the hook must restore
    // whatever the shell started with.
    let start = value("start");
    assert!(start.ends_with(BASE_PATH), "{start}");
    assert_eq!(value("a"), format!("{a}:{start}"));
    assert_eq!(value("node"), "managed-node");
    assert_eq!(value("nested"), start, "{stderr}");
    assert_eq!(value("b"), format!("{b}:{start}"));
    assert_eq!(value("stale"), format!("{b}:{start}"));
    assert_eq!(value("removed"), start);
    assert_eq!(value("off"), start);
    assert_eq!(value("end"), start);
    // Notices appear once on entering or changing state, never every prompt.
    assert_eq!(stderr.matches("turbo:").count(), 3, "{stderr}");
    assert_eq!(stderr.matches("not installed").count(), 2, "{stderr}");
    assert_eq!(stderr.matches("turbo.lock changed").count(), 1, "{stderr}");
}

#[test]
fn bash_hook_activates_only_trusted_set_up_repositories() {
    journey("bash");
}

#[test]
fn zsh_hook_activates_only_trusted_set_up_repositories() {
    journey("zsh");
}

#[test]
fn fish_hook_activates_only_trusted_set_up_repositories() {
    journey("fish");
}

#[test]
fn powershell_hook_activates_only_trusted_set_up_repositories() {
    journey("pwsh");
}
