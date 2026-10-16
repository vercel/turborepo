#![allow(clippy::unwrap_used)]
//! Native Windows locked fetches. A batch stand-in plays the managed cargo
//! (a rustup proxy in real installs) behind the private home and lock guards.

use super::*;

const LOCK: &str = "version = 4\n";
const TOOLCHAIN: &str = "1.99.0-x86_64-pc-windows-msvc";

struct Workspace {
    root: tempfile::TempDir,
    log: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".turbo")).unwrap();
        fs::write(root.path().join("Cargo.lock"), LOCK).unwrap();
        let log = root.path().join(".turbo").join("cargo-calls");
        Self { root, log }
    }

    /// A stand-in managed cargo running `body` after recording its call.
    fn cargo(&self, body: &str) -> PathBuf {
        let path = self.root.path().join(".turbo").join("cargo.cmd");
        let script = format!(
            "@echo off\r\n>>\"{}\" echo %*;%CARGO_HOME%;%RUSTUP_TOOLCHAIN%;%CD%\r\n{body}\r\n",
            self.log.display()
        );
        fs::write(&path, script).unwrap();
        path
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[test]
fn locked_fetch_uses_the_private_home_and_managed_env_and_honors_offline() {
    let workspace = Workspace::new();
    let root = workspace.root.path();
    let home = cargo_home(root).unwrap();
    assert_eq!(home, root.join(".turbo").join("cargo-home"));
    assert_eq!(cargo_home(root).unwrap(), home);
    let cargo = workspace.cargo("echo progress 1>&2");
    let env = BTreeMap::from([("RUSTUP_TOOLCHAIN".to_owned(), OsString::from(TOOLCHAIN))]);
    prefetch(root, &cargo, &home, false, &env).unwrap();
    prefetch(root, &cargo, &home, true, &env).unwrap();
    let calls = workspace.calls();
    let fields: Vec<Vec<_>> = calls.iter().map(|c| c.split(';').collect()).collect();
    assert_eq!(fields.len(), 2);
    for (call, args) in fields
        .iter()
        .zip(["fetch --locked", "fetch --locked --offline"])
    {
        let home = home.display().to_string();
        assert_eq!(call[..3], [args, home.as_str(), TOOLCHAIN]);
        assert!(same_file::is_same_file(call[3], root).unwrap(), "{call:?}");
    }
    assert_eq!(fs::read_to_string(root.join("Cargo.lock")).unwrap(), LOCK);
}

#[test]
fn stale_failed_rewritten_and_missing_locks_are_actionable() {
    let workspace = Workspace::new();
    let root = workspace.root.path();
    let home = cargo_home(root).unwrap();
    for (body, expected) in [
        (
            "echo error: the lock file Cargo.lock needs to be updated but --locked was passed to \
             prevent this 1>&2\r\nexit /b 101",
            "StaleLock",
        ),
        (
            "echo error: failed to download from `https://index.crates.io/` 1>&2\r\nexit /b 101",
            "Fetch(\"101\")",
        ),
        (">>Cargo.lock echo # rewritten", "LockChanged"),
    ] {
        fs::write(root.join("Cargo.lock"), LOCK).unwrap();
        let cargo = workspace.cargo(body);
        let error = prefetch(root, &cargo, &home, false, &BTreeMap::new()).unwrap_err();
        assert_eq!(format!("{error:?}"), expected);
    }
    fs::remove_file(root.join("Cargo.lock")).unwrap();
    let calls = workspace.calls().len();
    let cargo = workspace.cargo("exit /b 0");
    assert!(matches!(
        prefetch(root, &cargo, &home, false, &BTreeMap::new()),
        Err(Error::MissingLock)
    ));
    assert_eq!(workspace.calls().len(), calls);
}

#[test]
fn a_junctioned_cargo_home_is_rejected() {
    let workspace = Workspace::new();
    let root = workspace.root.path();
    let outside = tempfile::tempdir().unwrap();
    // Junctions need no symlink privilege, so any user can plant one.
    let status = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(root.join(".turbo").join("cargo-home"))
        .arg(outside.path())
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(matches!(cargo_home(root), Err(Error::UnsafeHome(_))));
}
