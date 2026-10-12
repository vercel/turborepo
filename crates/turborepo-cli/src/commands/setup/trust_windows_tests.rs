//! Windows user-state checks with fixture installations. Real Windows tool
//! generations and provisioning arrive separately; these tests exercise the
//! record, descriptor and identity checks that setup, run and the hook share.
#![allow(clippy::unwrap_used)]

use tempfile::TempDir;
use turborepo_tool_install::Current;

use super::*;
use crate::commands::setup::dacl::{
    self,
    tests::{icacls, junction},
};

const EVERYONE_READ: &[&str] = &["/grant", "*S-1-1-0:(R)"];

struct Fixture {
    home: TempDir,
    repo: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let (home, repo) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        fs::write(repo.path().join("turbo.lock"), "{}").unwrap();
        let tools = repo.path().join(".turbo").join("tools");
        fs::create_dir_all(tools.join("generation-a").join("bin")).unwrap();
        fs::write(tools.join("manifest.json"), "fixture inventory").unwrap();
        Self { home, repo }
    }

    fn trust(&self) -> TrustStore {
        TrustStore::new(self.home.path(), self.repo.path()).unwrap()
    }

    fn records(&self) -> PathBuf {
        self.home.path().join("turborepo").join("setup-roots")
    }

    /// Stands in for Store::current, which supplies the canonical bin path.
    fn installation(&self, trust: &TrustStore) -> Installation {
        let bin = trust.root.join(".turbo").join("tools").join("generation-a");
        let bin = bin.join("bin");
        let tools = vec![Tool {
            id: "node".into(),
            version: "24.0.0".into(),
            platform: "windows-x64".into(),
            artifact_sha256: "a".repeat(64),
            executables: [("node".into(), "node.exe".into())].into(),
        }];
        let current = Current {
            tools,
            bin,
            record: None,
        };
        trust
            .inspect_installation(|| Ok(Some(current)))
            .unwrap()
            .unwrap()
    }

    fn publish(&self) -> TrustStore {
        let trust = self.trust();
        trust.preflight().unwrap();
        trust.write_record(self.installation(&trust)).unwrap();
        trust
    }
}

#[test]
fn records_are_owner_only_and_round_trip_with_the_path_spelling() {
    let f = Fixture::new();
    let trust = f.publish();
    let record = trust.load().unwrap().unwrap();
    assert!(record.installation == f.installation(&trust));
    assert_eq!(record.lock_sha256, hash(b"{}"));
    assert!(
        trust
            .root
            .as_os_str()
            .to_string_lossy()
            .starts_with(r"\\?\")
    );
    assert!(
        !record
            .installation
            .bin
            .to_string_lossy()
            .starts_with(r"\\?\")
    );
    assert!(trust.path().starts_with(f.records()));
    // Verified from the live security descriptors, not from path metadata.
    dacl::check(&dacl::open(&f.records(), true).unwrap()).unwrap();
    dacl::check(&dacl::open(&trust.path(), false).unwrap()).unwrap();
    // The records directory is protected, so parent grants never reach it.
    let shared = f.home.path().join("turborepo");
    assert!(icacls(&shared, &["/grant", "*S-1-1-0:(OI)(CI)(M)"]));
    assert!(trust.load().unwrap().is_some());
    let bytes = fs::read(trust.path()).unwrap();
    trust.write_record(f.installation(&trust)).unwrap();
    assert_eq!(fs::read(trust.path()).unwrap(), bytes);
}

#[test]
fn lock_changes_moved_roots_and_same_path_reclones_do_not_match() {
    let f = Fixture::new();
    let trust = f.publish();
    fs::write(f.repo.path().join("turbo.lock"), "{ }").unwrap();
    let lock = read(&trust.root.join("turbo.lock"), false)
        .unwrap()
        .unwrap();
    assert_ne!(trust.load().unwrap().unwrap().lock_sha256, hash(&lock));

    let moved = f.home.path().join("moved");
    fs::rename(f.repo.path(), &moved).unwrap();
    assert!(
        TrustStore::new(f.home.path(), &moved)
            .unwrap()
            .load()
            .unwrap()
            .is_none()
    );
    // A new directory at the old path has a new NTFS file ID.
    fs::create_dir(f.repo.path()).unwrap();
    fs::rename(moved.join(".turbo"), f.repo.path().join(".turbo")).unwrap();
    let reclone = f.trust();
    assert_eq!(reclone.path(), trust.path());
    assert!(reclone.load().unwrap().unwrap().installation != f.installation(&reclone));
}

#[test]
fn unsafe_windows_user_state_fails_closed_without_following_links() {
    for case in 0..6 {
        let f = Fixture::new();
        let trust = if case == 5 {
            // A directory created by anyone else is never adopted.
            fs::create_dir_all(f.records()).unwrap();
            assert!(icacls(&f.records(), EVERYONE_READ));
            f.trust()
        } else {
            f.publish()
        };
        let record = trust.path();
        match case {
            0 => assert!(icacls(&record, EVERYONE_READ)),
            1 => assert!(icacls(&f.records(), EVERYONE_READ)),
            2 => assert!(icacls(&f.records(), &["/grant", "*S-1-1-0:(OI)(IO)(R)"])),
            3 => fs::hard_link(&record, f.repo.path().join("committed-record")).unwrap(),
            4 => {
                let elsewhere = f.home.path().join("elsewhere");
                fs::rename(f.records(), &elsewhere).unwrap();
                junction(&f.records(), &elsewhere);
            }
            _ => {
                let installation = f.installation(&trust);
                assert!(trust.write_record(installation).is_err());
                assert_eq!(fs::read_dir(f.records()).unwrap().count(), 0);
            }
        }
        assert!(trust.preflight().is_err(), "case {case}");
        assert!(trust.load().is_err(), "case {case}");
    }
}

#[test]
fn record_symlinks_are_rejected_when_the_host_can_create_them() {
    let f = Fixture::new();
    let trust = f.publish();
    let target = f.home.path().join("target.json");
    fs::rename(trust.path(), &target).unwrap();
    // File symlinks need Developer Mode or elevation, which CI runners have.
    if std::os::windows::fs::symlink_file(&target, trust.path()).is_ok() {
        assert!(trust.load().is_err());
    }
}
