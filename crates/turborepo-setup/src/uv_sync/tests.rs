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
    fn run(&self, offline: bool) -> Result<(), Error> {
        UvSync {
            uv: &self.temp.path().join("uv"),
            root: &self.root(),
            offline,
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
    f.run(false).unwrap();
    assert!(f.root().join(".venv/synced").is_file());
    fs::remove_dir_all(f.root().join(".venv")).unwrap();
    f.run(true).unwrap();
    assert!(f.root().join(".venv/synced").is_file());
    assert_eq!(
        f.calls(),
        [
            "sync --locked --all-packages",
            "sync --locked --all-packages --offline"
        ]
    );
    assert_eq!(f.lock(), LOCK);
}

#[test]
fn native_build_settings_reach_uv_without_setup_flags() {
    let f = Fixture::new("printf '%s' \"$UV_NO_BUILD\" > native-build-setting");
    for value in ["true", "false"] {
        let env = BTreeMap::from([("UV_NO_BUILD".into(), OsString::from(value))]);
        UvSync {
            uv: &f.temp.path().join("uv"),
            root: &f.root(),
            offline: false,
            env: &env,
        }
        .run()
        .unwrap();
        assert_eq!(
            fs::read_to_string(f.root().join("native-build-setting")).unwrap(),
            value
        );
    }
    assert_eq!(f.calls(), ["sync --locked --all-packages"; 2]);
    assert_eq!(f.lock(), LOCK);
}

#[test]
fn missing_stale_or_rewritten_locks_are_rejected_without_rewriting() {
    let f = Fixture::new("exit 0");
    fs::remove_file(f.root().join("uv.lock")).unwrap();
    assert!(matches!(f.run(false), Err(Error::MissingLock)));
    assert!(f.calls().is_empty());
    assert!(!f.root().join("uv.lock").exists());

    // uv rejects a stale lock under --locked; an interrupted sync also fails.
    let f = Fixture::new("echo 'lockfile needs to be updated' >&2; exit 2");
    assert!(matches!(f.run(false), Err(Error::Failed(2))));
    f.mock("kill -INT $$");
    assert!(matches!(f.run(false), Err(Error::Failed(-1))));
    assert_eq!(f.lock(), LOCK);
    // A later sync repairs the environment after either failure.
    f.mock("mkdir -p .venv");
    f.run(false).unwrap();

    let f = Fixture::new("echo 'version = 2' > uv.lock");
    assert!(matches!(f.run(false), Err(Error::LockChanged)));
    assert_eq!(f.lock(), LOCK);
}
