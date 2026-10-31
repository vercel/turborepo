//! Real bash, zsh, fish, and PowerShell sessions driving `turbo hook` at each
//! prompt.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use turborepo_cli::test_support::RecordedInstallation;
use turborepo_tool_install::{Tool, UserStore};

// A duplicate entry proves the hook removes only what it added.
const BASE_PATH: &str = "/usr/bin:/bin:/usr/bin";

fn user_state(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        home.join("data")
    }
}

/// The tool store `turbo` locates under `home`.
fn user_store(home: &Path, root: &Path) -> UserStore {
    UserStore::at(&user_state(home).join("turborepo/tools"), root).unwrap()
}

/// A recorded tool exporting `bin/<id>`, with the variables its adapter
/// records for a `linux-x64-gnu` selection, the same on every host.
fn tool(id: &str) -> Tool {
    use turborepo_tool_install::{ManagedVar as V, TreeRef, VarValue};
    let tree = |id: &str| VarValue::Tree(TreeRef::new(id, ""));
    let word = |word: &str| VarValue::Word(word.into());
    let uv = [
        (V::UvPythonDownloads, word("never")),
        (V::UvPythonInstallDir, tree("python")),
    ];
    let (version, vars) = match id {
        "go" => (
            "1.23.0",
            vec![(V::Goroot, tree("go")), (V::Gotoolchain, word("local"))],
        ),
        "python" => {
            let mut vars = uv.to_vec();
            vars.push((V::UvPythonPreference, word("only-managed")));
            vars.push((V::UvPython, word("cpython-3.12.12-linux-x86_64-gnu")));
            ("3.12.12", vars)
        }
        "rust" => (
            "1.90.0",
            vec![
                (V::CargoHome, VarValue::RepoCargoHome),
                (V::RustupHome, tree("rust")),
                (V::RustupToolchain, word("1.90.0-x86_64-unknown-linux-gnu")),
            ],
        ),
        "uv" => ("0.9.0", uv.to_vec()),
        _ => ("24.0.0", Vec::new()),
    };
    Tool {
        id: id.into(),
        version: version.into(),
        platform: "linux-x64-gnu".into(),
        artifact_sha256: "a".repeat(64),
        executables: [(id.into(), format!("bin/{id}").into())].into(),
        vars: vars.into_iter().collect(),
    }
}

/// A clone with its own Git boundary. `home` set-up repositories get a real
/// installation of `tools` (sorted ids) in the user's tool store plus the
/// record `turbo setup` publishes for it.
fn repo(
    dir: &Path,
    flagged: bool,
    home: Option<&Path>,
    tools: &[&str],
) -> Option<RecordedInstallation> {
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
    let tools: Vec<_> = tools.iter().map(|id| tool(id)).collect();
    let store = user_state(home).join("turborepo/tools");
    turborepo_cli::test_support::install_tools_for_tests(
        &root,
        Some(&store),
        tools,
        None,
        |tool, tree| {
            let export = tree.join("bin").join(&tool.id);
            fs::create_dir_all(tree.join("bin"))?;
            fs::write(&export, format!("#!/bin/sh\necho managed-{}\n", tool.id))?;
            fs::set_permissions(&export, fs::Permissions::from_mode(0o755))?;
            Ok(())
        },
    )
    .unwrap();
    turborepo_cli::test_support::recorded_installation_for_tests(&root, &store).unwrap()
}

/// The environment directory `installed` names, which `Step::RemoveTools`
/// deletes through `REMOVE`.
fn env_dir(installed: &RecordedInstallation) -> String {
    installed.bin.parent().unwrap().to_str().unwrap().to_owned()
}

/// The `bin` a repository's hook activation prepends.
fn bin(installed: &RecordedInstallation) -> String {
    installed.bin.to_str().unwrap().to_owned()
}

/// One shell-neutral command per prompt; each shell renders its own syntax.
enum Step {
    Cd(&'static str),
    Show(&'static str),
    /// PATH and every runtime variable the hook manages.
    Vars(&'static str),
    Set(&'static str, &'static str),
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
        Step::Vars(key) => format!("echo '@{key}'; /usr/bin/env"),
        Step::Set(name, value) if pwsh => format!("$env:{name} = '{value}'"),
        Step::Set(name, value) if shell == "fish" => format!("set -gx {name} {value}"),
        Step::Set(name, value) => format!("export {name}={value}"),
        Step::Node if pwsh => "\"node=$(node)\"".into(),
        Step::Node if shell == "fish" => "echo \"node=\"(node)".into(),
        Step::Node => "echo \"node=$(node)\"".into(),
        Step::TouchLock if pwsh => "Add-Content turbo.lock ' '".into(),
        Step::TouchLock => "printf ' ' >> turbo.lock".into(),
        Step::RemoveTools if pwsh => "Remove-Item -Recurse -Force $env:REMOVE".into(),
        Step::RemoveTools => "rm -rf \"$REMOVE\"".into(),
        Step::Nothing if pwsh => "$null = 0".into(),
        Step::Nothing => "true".into(),
    }
}

/// Runs `steps` in a real `shell` started in `<temp>/plain` with the user's
/// own `vars`, firing the prompt hook after each step. None when the shell is
/// not installed.
fn session(
    shell: &str,
    temp: &Path,
    vars: &[(&str, &str)],
    steps: &[Step],
) -> Option<(String, String)> {
    let Ok(program) = which::which(shell) else {
        eprintln!("skipping: {shell} is not installed");
        return None;
    };
    let home = temp.join("home");
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
    for step in steps {
        script.push_str(&format!("{}{prompt}\n", line(shell, step)));
    }
    script.push_str("exit\n");
    let mut child = Command::new(program)
        .args(args)
        .current_dir(temp.join("plain"))
        .env_clear()
        .envs(vars.iter().copied())
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
    Some((
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    ))
}

fn journey(shell: &str) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let node = &["node"];
    let a = repo(&temp.path().join("a"), true, Some(&home), node).unwrap();
    let b = repo(&temp.path().join("b"), true, Some(&home), node).unwrap();
    // The same selection shares one environment.
    assert_eq!(a.bin, b.bin);
    let remove = env_dir(&b);
    let (a, b) = (bin(&a), bin(&b));
    // Nested, flagged, never set up: untrusted. Flag off: ignored even if set up.
    repo(&temp.path().join("a/vendor/c"), true, None, node);
    repo(&temp.path().join("off"), false, Some(&home), node);
    fs::create_dir(temp.path().join("plain")).unwrap();
    let Some((stdout, stderr)) = session(shell, temp.path(), &[("REMOVE", &remove)], STEPS) else {
        return;
    };
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

// The user's own values; the rest start unset.
const USER_VARS: &[(&str, &str)] = &[
    ("GOROOT", "/mine/go"),
    ("RUSTUP_TOOLCHAIN", "stable"),
    ("UV_PYTHON", "3.11"),
];

const RUNTIME_STEPS: &[Step] = &[
    Step::Vars("start"),
    Step::Cd("../r"),
    Step::Vars("r"),
    Step::Set("GOTOOLCHAIN", "go1.99"),
    // Each lock change re-evaluates the repository.
    Step::TouchLock,
    Step::Vars("stale"),
    Step::TouchLock,
    Step::Vars("again"),
    Step::Cd("../s"),
    Step::Vars("s"),
    Step::RemoveTools,
    Step::Vars("removed"),
    Step::Cd("../plain"),
    Step::Vars("end"),
];

/// What `Step::Vars(key)` printed, with whether the hook's state records
/// runtime variables.
fn printed(stdout: &str, key: &str) -> (BTreeMap<String, String>, bool) {
    let marker = format!("@{key}");
    let mut lines = stdout.lines().skip_while(|line| *line != marker);
    assert!(lines.next().is_some(), "{key} missing\n{stdout}");
    let mut vars: BTreeMap<_, _> = lines
        .take_while(|line| !line.starts_with('@'))
        .filter_map(|line| line.split_once('='))
        .filter(|(name, _)| {
            ["PATH", "__TURBO_HOOK_STATE"].contains(name)
                || turborepo_tool_install::ManagedVar::from_name(name).is_some()
        })
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    // The state's fourth field records the runtime variables the hook manages.
    let saved = vars
        .remove("__TURBO_HOOK_STATE")
        .is_some_and(|state| state.split(';').nth(3).is_some_and(|f| !f.is_empty()));
    (vars, saved)
}

fn runtime_journey(shell: &str) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let all = &["go", "node", "python", "rust", "uv"];
    let r_installed = repo(&temp.path().join("r"), true, Some(&home), all).unwrap();
    let s_installed = repo(&temp.path().join("s"), true, Some(&home), &["go"]).unwrap();
    let (r, s) = (bin(&r_installed), bin(&s_installed));
    fs::create_dir(temp.path().join("plain")).unwrap();
    let mut session_vars = USER_VARS.to_vec();
    let remove = env_dir(&s_installed);
    session_vars.push(("REMOVE", &remove));
    let Some((stdout, stderr)) = session(shell, temp.path(), &session_vars, RUNTIME_STEPS) else {
        return;
    };
    let vars = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        let pairs = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()));
        pairs.collect()
    };
    let (start, saved) = printed(&stdout, "start");
    assert!(!saved);
    let path = start["PATH"].clone();
    let mut user = vars(USER_VARS);
    user.insert("PATH".into(), path.clone());
    assert_eq!(start, user, "{stderr}");

    // Exactly what managed run sets for these tools, over the user's values.
    let tree = |installed: &RecordedInstallation, id: &str| {
        installed.tree(id).to_str().unwrap().to_owned()
    };
    let root = temp.path().join("r").canonicalize().unwrap();
    let cargo_home = user_store(&home, &root).cargo_home(&root);
    let cargo_home = cargo_home.to_str().unwrap();
    let managed = vars(&[
        ("PATH", &format!("{r}:{path}")),
        ("CARGO_HOME", cargo_home),
        ("GOROOT", &tree(&r_installed, "go")),
        ("GOTOOLCHAIN", "local"),
        ("RUSTUP_HOME", &tree(&r_installed, "rust")),
        ("RUSTUP_TOOLCHAIN", "1.90.0-x86_64-unknown-linux-gnu"),
        ("UV_PYTHON", "cpython-3.12.12-linux-x86_64-gnu"),
        ("UV_PYTHON_DOWNLOADS", "never"),
        ("UV_PYTHON_INSTALL_DIR", &tree(&r_installed, "python")),
        ("UV_PYTHON_PREFERENCE", "only-managed"),
    ]);
    assert_eq!(printed(&stdout, "r"), (managed.clone(), true), "{stderr}");
    // Stale tools keep their variables. The user's own GOTOOLCHAIN, set
    // inside, survives every later evaluation of this repository.
    let mut edited = managed;
    edited.insert("GOTOOLCHAIN".into(), "go1.99".into());
    assert_eq!(printed(&stdout, "stale"), (edited.clone(), true));
    assert_eq!(printed(&stdout, "again"), (edited, true));

    // Straight to another repository: r's variables are restored first.
    let mut only_go = user.clone();
    only_go.insert("PATH".into(), format!("{s}:{path}"));
    only_go.insert("GOROOT".into(), tree(&s_installed, "go"));
    only_go.insert("GOTOOLCHAIN".into(), "local".into());
    assert_eq!(printed(&stdout, "s"), (only_go, true), "{stderr}");
    // Leaving every managed variable restores the shell exactly, keeping the
    // value the user set inside.
    user.insert("GOTOOLCHAIN".into(), "go1.99".into());
    assert_eq!(printed(&stdout, "removed"), (user.clone(), false));
    assert_eq!(printed(&stdout, "end"), (user, false));
    assert_eq!(stderr.matches("turbo:").count(), 2, "{stderr}");
    assert_eq!(stderr.matches("turbo.lock changed").count(), 1, "{stderr}");
    assert_eq!(stderr.matches("not installed").count(), 1, "{stderr}");
}

#[test]
fn bash_hook_sets_and_restores_runtime_variables() {
    runtime_journey("bash");
}

#[test]
fn zsh_hook_sets_and_restores_runtime_variables() {
    runtime_journey("zsh");
}

#[test]
fn fish_hook_sets_and_restores_runtime_variables() {
    runtime_journey("fish");
}

#[test]
fn powershell_hook_sets_and_restores_runtime_variables() {
    runtime_journey("pwsh");
}
