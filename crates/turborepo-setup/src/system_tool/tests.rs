#![allow(clippy::unwrap_used)]

use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt};

use super::*;

struct Host {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    bin: PathBuf,
}

impl Host {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let bin = temp.path().join("bin");
        fs::create_dir_all(repo.join("bin")).unwrap();
        fs::create_dir_all(&bin).unwrap();
        Self {
            _temp: temp,
            repo,
            bin,
        }
    }

    fn tool(&self, dir: &Path, name: &str, script: &str) {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn path(&self) -> OsString {
        std::env::join_paths([
            PathBuf::from("relative"),
            self.repo.join("bin"),
            self.bin.clone(),
        ])
        .unwrap()
    }
}

fn lock(version: &str) -> Lock {
    let json = format!(
        r#"{{"schemaVersion":0,"tools":{{"node":{{"adapter":"node","version":"{version}",
            "declarations":[{{"file":".nvmrc","request":"{version}"}}],
            "installation":{{"kind":"verify-system","executables":["node"]}}}}}}}}"#
    );
    Lock::parse(json.as_bytes()).unwrap()
}

#[test]
fn repository_and_relative_path_entries_never_supply_tools() {
    let host = Host::new();
    host.tool(&host.repo.join("bin"), "node", "echo v99.0.0");
    let path = host.path();
    assert!(matches!(
        probe("node", Some(&path), &host.repo),
        Err(Error::Missing(_))
    ));
    // A non-executable file is not a tool either.
    fs::write(host.bin.join("node"), "not executable").unwrap();
    assert_eq!(find("node", Some(&path), &host.repo), None);
    host.tool(&host.bin, "node", "echo v24.0.0");
    let (found, version) = probe("node", Some(&path), &host.repo).unwrap();
    assert_eq!(found, host.bin.join("node"));
    assert_eq!(version, Version::new(24, 0, 0));
}

#[test]
fn unreadable_versions_fail_instead_of_passing() {
    let host = Host::new();
    let path = host.path();
    for script in ["echo not-a-version", "echo v24.0.0; exit 3", "exit 0"] {
        host.tool(&host.bin, "node", script);
        assert!(
            matches!(
                probe("node", Some(&path), &host.repo),
                Err(Error::Probe { .. })
            ),
            "{script}"
        );
    }
}

#[test]
fn verification_requires_the_exact_locked_version() {
    let host = Host::new();
    let path = host.path();
    host.tool(&host.bin, "node", "echo v24.0.0");
    assert_eq!(
        verify(&lock("24.0.0"), Some(&path), &host.repo).unwrap(),
        [Verified {
            id: "node".into(),
            version: "24.0.0".into(),
            path: host.bin.join("node"),
        }]
    );
    let error = verify(&lock("24.1.0"), Some(&path), &host.repo).unwrap_err();
    assert!(matches!(error, Error::Mismatch { .. }), "{error}");
    assert!(error.to_string().contains("requires 24.1.0"), "{error}");
    assert!(matches!(
        verify(&lock("24.0.0"), None, &host.repo),
        Err(Error::Missing(_))
    ));
}
