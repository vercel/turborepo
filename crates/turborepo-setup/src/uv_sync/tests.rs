#![allow(clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;

use super::*;

const LOCK: &str = "version = 1\n";

struct Fixture {
    temp: tempfile::TempDir,
}
impl Fixture {
    /// A mock managed uv: records its arguments, then runs `body`.
    fn new(body: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("repo")).unwrap();
        fs::write(temp.path().join("repo/uv.lock"), LOCK).unwrap();
        let f = Self { temp };
        f.mock(body);
        f
    }
    fn mock(&self, body: &str) {
        let uv = self.temp.path().join("uv");
        let record = self.temp.path().join("args");
        fs::write(
            &uv,
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n{body}\n", record.display()),
        )
        .unwrap();
        fs::set_permissions(&uv, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn root(&self) -> std::path::PathBuf {
        self.temp.path().join("repo")
    }
    fn run(&self, offline: bool, builds: Builds) -> Result<(), Error> {
        UvSync {
            uv: &self.temp.path().join("uv"),
            root: &self.root(),
            offline,
            builds,
            env: &BTreeMap::new(),
        }
        .run()
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.temp.path().join("args"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    fn lock(&self) -> String {
        fs::read_to_string(self.root().join("uv.lock")).unwrap()
    }
}

#[test]
fn locked_sync_repeats_safely_and_recreates_a_removed_environment() {
    let f = Fixture::new("mkdir -p .venv && touch .venv/synced");
    f.run(false, Builds::Native).unwrap();
    assert!(f.root().join(".venv/synced").is_file());
    fs::remove_dir_all(f.root().join(".venv")).unwrap();
    f.run(true, Builds::Skip).unwrap();
    assert!(f.root().join(".venv/synced").is_file());
    assert_eq!(
        f.calls(),
        [
            "sync --locked --all-packages",
            "sync --locked --all-packages --offline --no-build"
        ]
    );
    assert_eq!(f.lock(), LOCK);
}

#[test]
fn missing_stale_or_rewritten_locks_are_rejected_without_rewriting() {
    let f = Fixture::new("exit 0");
    fs::remove_file(f.root().join("uv.lock")).unwrap();
    assert!(matches!(
        f.run(false, Builds::Native),
        Err(Error::MissingLock)
    ));
    assert!(f.calls().is_empty());
    assert!(!f.root().join("uv.lock").exists());

    // uv rejects a stale lock under --locked; an interrupted sync also fails.
    let f = Fixture::new("echo 'lockfile needs to be updated' >&2; exit 2");
    assert!(matches!(
        f.run(false, Builds::Native),
        Err(Error::Failed(2))
    ));
    f.mock("kill -INT $$");
    assert!(matches!(
        f.run(false, Builds::Native),
        Err(Error::Failed(-1))
    ));
    assert_eq!(f.lock(), LOCK);
    // A later sync repairs the environment after either failure.
    f.mock("mkdir -p .venv");
    f.run(false, Builds::Native).unwrap();

    let f = Fixture::new("echo 'version = 2' > uv.lock");
    assert!(matches!(
        f.run(false, Builds::Native),
        Err(Error::LockChanged)
    ));
    assert_eq!(f.lock(), LOCK);
}

#[test]
fn disclosure_describes_uv_builds_not_javascript_lifecycle_scripts() {
    let f = Fixture::new("exit 0");
    let env = BTreeMap::new();
    let sync = |builds| UvSync {
        uv: Path::new("uv"),
        root: f.temp.path(),
        offline: false,
        builds,
        env: &env,
    };
    assert!(sync(Builds::Native).disclosure().contains("build backends"));
    assert!(sync(Builds::Skip).disclosure().contains("--no-build"));
}
