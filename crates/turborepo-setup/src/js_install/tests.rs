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
