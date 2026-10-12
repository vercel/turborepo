#![allow(clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;

use super::*;

struct Fixture {
    temp: tempfile::TempDir,
    frozen: Frozen,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("repo")).unwrap();
        Self {
            temp,
            frozen: Frozen::new(Manager::Npm).unwrap(),
        }
    }
    fn root(&self) -> std::path::PathBuf {
        self.temp.path().join("repo")
    }
    /// A mock managed npm that runs `body` in the repository.
    fn npm(&self, body: &str) -> std::path::PathBuf {
        let npm = self.temp.path().join("npm");
        fs::write(&npm, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&npm, fs::Permissions::from_mode(0o755)).unwrap();
        npm
    }
}

#[test]
fn missing_lock_is_an_error_not_permission_to_create_one() {
    let f = Fixture::new();
    let error = f.frozen.lockfile(&f.root()).unwrap_err().to_string();
    assert!(
        error.contains("npm-shrinkwrap.json or package-lock.json") && error.contains("npm install"),
        "{error}"
    );
    fs::write(f.root().join("package-lock.json"), "{}").unwrap();
    assert_eq!(f.frozen.lockfile(&f.root()).unwrap(), "package-lock.json");
    // npm ci prefers a shrinkwrap.
    fs::write(f.root().join("npm-shrinkwrap.json"), "{}").unwrap();
    assert_eq!(f.frozen.lockfile(&f.root()).unwrap(), "npm-shrinkwrap.json");
    assert!(Frozen::new(Manager::Aube).is_none());
}

#[test]
fn pnpm_installs_frozen_from_pnpm_lock_yaml() {
    let f = Fixture::new();
    let pnpm = Frozen::new(Manager::Pnpm).unwrap();
    let error = pnpm.lockfile(&f.root()).unwrap_err().to_string();
    assert!(
        error.contains("committed pnpm-lock.yaml") && error.contains("`pnpm install`"),
        "{error}"
    );
    fs::write(f.root().join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
    assert_eq!(pnpm.lockfile(&f.root()).unwrap(), "pnpm-lock.yaml");
    let args = pnpm.args(&Version::new(10, 0, 0));
    assert_eq!(args, ["install", "--frozen-lockfile"]);
    let rewrite = f.npm("echo rewritten > pnpm-lock.yaml");
    let error = pnpm.run(&rewrite, args, &f.root(), &BTreeMap::new());
    assert!(matches!(error, Err(Error::LockChanged { .. })));
    let lock = fs::read(f.root().join("pnpm-lock.yaml")).unwrap();
    assert_eq!(lock, b"lockfileVersion: '9.0'\n");
}

#[test]
fn runs_the_frozen_command_with_the_given_environment_and_returns_its_status() {
    let f = Fixture::new();
    fs::write(f.root().join("package-lock.json"), "lock").unwrap();
    let npm = f.npm(r#"echo "$* $MANAGED" > ran; exit "$CODE""#);
    let args = f.frozen.args(&Version::new(11, 0, 0));
    for code in [0, 7] {
        let env = BTreeMap::from([
            ("MANAGED".to_owned(), OsString::from("yes")),
            ("CODE".to_owned(), OsString::from(code.to_string())),
        ]);
        let status = f.frozen.run(&npm, args, &f.root(), &env).unwrap();
        assert_eq!(status.code(), Some(code));
        let ran = fs::read_to_string(f.root().join("ran")).unwrap();
        assert_eq!(ran, "ci yes\n");
    }
    assert_eq!(
        fs::read(f.root().join("package-lock.json")).unwrap(),
        b"lock"
    );
}

#[test]
fn rewritten_or_created_lockfiles_are_restored_and_fail() {
    for body in [
        "echo rewritten > package-lock.json",
        "echo created > npm-shrinkwrap.json",
    ] {
        let f = Fixture::new();
        fs::write(f.root().join("package-lock.json"), "lock").unwrap();
        let npm = f.npm(body);
        let error = f.frozen.run(&npm, &["ci"], &f.root(), &BTreeMap::new());
        let message = error.unwrap_err().to_string();
        assert!(
            message.contains("`npm ci` changed the committed") && message.contains("restored"),
            "{message}"
        );
        assert_eq!(
            fs::read(f.root().join("package-lock.json")).unwrap(),
            b"lock"
        );
        assert!(!f.root().join("npm-shrinkwrap.json").exists());
    }
}

#[test]
fn yarn_classic_and_modern_refuse_lock_updates_their_own_way() {
    let f = Fixture::new();
    let yarn = Frozen::new(Manager::Yarn).unwrap();
    let error = yarn.lockfile(&f.root()).unwrap_err().to_string();
    assert!(error.contains("committed yarn.lock"), "{error}");
    fs::write(f.root().join("yarn.lock"), "# yarn lockfile v1\n").unwrap();
    assert_eq!(yarn.lockfile(&f.root()).unwrap(), "yarn.lock");
    let classic = yarn.args(&Version::new(1, 22, 22));
    assert_eq!(classic, ["install", "--frozen-lockfile"]);
    assert_eq!(
        yarn.args(&Version::new(4, 5, 0)),
        ["install", "--immutable"]
    );
}

#[test]
fn bun_installs_frozen_from_either_lock_and_never_migrates_it() {
    let f = Fixture::new();
    let bun = Frozen::new(Manager::Bun).unwrap();
    let error = bun.lockfile(&f.root()).unwrap_err().to_string();
    assert!(error.contains("committed bun.lock or bun.lockb"), "{error}");
    fs::write(f.root().join("bun.lockb"), "binary").unwrap();
    assert_eq!(bun.lockfile(&f.root()).unwrap(), "bun.lockb");
    let args = bun.args(&Version::new(1, 2, 23));
    assert_eq!(args, ["install", "--frozen-lockfile"]);
    // A text lock written beside the committed binary one is removed.
    let migrate = f.npm("echo text > bun.lock");
    let error = bun.run(&migrate, args, &f.root(), &BTreeMap::new());
    assert!(matches!(error, Err(Error::LockChanged { .. })));
    assert!(!f.root().join("bun.lock").exists());
    assert_eq!(fs::read(f.root().join("bun.lockb")).unwrap(), b"binary");
}
