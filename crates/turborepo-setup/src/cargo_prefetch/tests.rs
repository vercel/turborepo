#![allow(clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;

use super::*;

const LOCK: &str = "version = 4\n";

struct Workspace {
    root: tempfile::TempDir,
    log: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".turbo")).unwrap();
        fs::write(root.path().join("Cargo.lock"), LOCK).unwrap();
        let log = root.path().join(".turbo/cargo-calls");
        Self { root, log }
    }

    /// A stand-in managed cargo running `body` after recording its call.
    fn cargo(&self, body: &str) -> PathBuf {
        let path = self.root.path().join(".turbo/cargo");
        let script = format!(
            "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$*\" \"$CARGO_HOME\" \"$(pwd -P)\" >> \
             '{}'\n{body}\n",
            self.log.display()
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
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
fn locked_fetch_uses_the_private_home_and_honors_offline() {
    let workspace = Workspace::new();
    let root = workspace.root.path();
    let home = cargo_home(root).unwrap();
    let mode = fs::metadata(&home).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    assert_eq!(cargo_home(root).unwrap(), home);
    let cargo = workspace.cargo("echo progress >&2");
    prefetch(root, &cargo, &home, false, &BTreeMap::new()).unwrap();
    prefetch(root, &cargo, &home, true, &BTreeMap::new()).unwrap();
    let cwd = root.canonicalize().unwrap();
    assert_eq!(
        workspace.calls(),
        [
            format!("fetch --locked|{}|{}", home.display(), cwd.display()),
            format!(
                "fetch --locked --offline|{}|{}",
                home.display(),
                cwd.display()
            ),
        ]
    );
    assert_eq!(fs::read_to_string(root.join("Cargo.lock")).unwrap(), LOCK);
}

#[test]
fn missing_stale_and_unreachable_locks_are_actionable() {
    let workspace = Workspace::new();
    let root = workspace.root.path();
    let home = cargo_home(root).unwrap();
    for (body, expected) in [
        (
            "echo 'error: the lock file Cargo.lock needs to be updated but --locked was passed to \
             prevent this' >&2; exit 101",
            "StaleLock",
        ),
        (
            "echo 'error: cannot update the lock file Cargo.lock because --locked was passed to \
             prevent this' >&2; exit 101",
            "StaleLock",
        ),
        (
            "echo 'error: failed to download from `https://index.crates.io/`' >&2; exit 101",
            "Fetch(\"101\")",
        ),
        ("echo '# rewritten' >> Cargo.lock", "LockChanged"),
    ] {
        fs::write(root.join("Cargo.lock"), LOCK).unwrap();
        let error =
            prefetch(root, &workspace.cargo(body), &home, false, &BTreeMap::new()).unwrap_err();
        assert_eq!(format!("{error:?}"), expected);
        assert!(error.to_string().contains("Cargo.lock") || expected.starts_with("Fetch"));
    }
    fs::remove_file(root.join("Cargo.lock")).unwrap();
    let calls = workspace.calls().len();
    let cargo = workspace.cargo("exit 0");
    assert!(matches!(
        prefetch(root, &cargo, &home, false, &BTreeMap::new()),
        Err(Error::MissingLock)
    ));
    assert_eq!(workspace.calls().len(), calls);
}

#[test]
fn a_redirected_cargo_home_is_rejected() {
    let workspace = Workspace::new();
    let root = workspace.root.path();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join(".turbo/cargo-home")).unwrap();
    assert!(matches!(cargo_home(root), Err(Error::UnsafeHome(_))));
}
