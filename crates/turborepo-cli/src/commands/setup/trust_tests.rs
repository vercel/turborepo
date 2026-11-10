use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use turborepo_tool_install::UserStore;

use super::*;
use crate::commands::setup::trust::TrustStore;

fn trust(f: &Fixture) -> TrustStore {
    let store = UserStore::at(f.owned.home(), f.owned.root()).unwrap();
    TrustStore::new(&store, f.owned.root()).unwrap()
}
fn record(f: &Fixture) -> (PathBuf, Value) {
    let path = fs::read_dir(f.owned.home().join("repos"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("record.json");
    let value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    (path, value)
}
/// The environment setup recorded, by its directory name.
fn env(f: &Fixture) -> String {
    let bin = Installed::open(&f.owned).current().unwrap().unwrap().bin;
    let env = bin.parent().unwrap().file_name().unwrap();
    env.to_str().unwrap().to_owned()
}

#[test]
fn fresh_setup_records_exact_private_external_evidence_and_repeat_is_noop() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let (path, value) = record(&f);
    assert!(!path.starts_with(f.owned.root()));
    let root = fs::metadata(f.owned.root()).unwrap();
    assert_eq!(value["schema"], 2);
    assert_eq!(value["root"], f.owned.root().to_str().unwrap());
    assert_eq!(value["device"], root.dev());
    assert_eq!(value["inode"], root.ino());
    assert_eq!(
        value["lock_sha256"],
        format!(
            "{:x}",
            Sha256::digest(fs::read(f.owned.root().join("turbo.lock")).unwrap())
        )
    );
    assert_eq!(value["env"], env(&f));
    assert!(value.get("selection").is_none());
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, FROZEN).unwrap();
    assert_eq!(empty.hits(), 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    f.no_execution();
}

#[test]
fn recorded_versions_are_the_installed_lock_versions_until_the_record_goes() {
    let f = Fixture::new();
    assert_eq!(trust(&f).recorded_versions(), None);
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    let locked: std::collections::BTreeMap<String, String> = lock["tools"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, tool)| (id.clone(), tool["version"].as_str().unwrap().to_owned()))
        .collect();
    let recorded = trust(&f).recorded_versions().unwrap();
    assert!(!recorded.is_empty());
    for (id, version) in &recorded {
        assert_eq!(locked.get(id), Some(version), "{id}");
    }
    // A record for another directory identity says nothing about this one.
    let (path, mut value) = record(&f);
    value["inode"] = Value::from(0);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(trust(&f).recorded_versions(), None);
}

#[test]
fn pre_store_repository_tools_are_never_read_and_setup_removes_them() {
    let f = Fixture::new();
    let old = f.owned.root().join(".turbo/tools/generation-old");
    fs::create_dir_all(old.join("bin")).unwrap();
    fs::write(old.join("bin/git"), "#!/bin/sh\ntouch probe-ran\n").unwrap();
    fs::set_permissions(old.join("bin/git"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        f.owned.root().join(".turbo/tools/manifest.json"),
        r#"{"schema":1,"generation":"generation-old","tools":[]}"#,
    )
    .unwrap();
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    assert!(!f.owned.root().join(".turbo/tools").exists());
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    assert!(!current.bin.join("git").exists());
    f.no_execution();
}

#[test]
fn a_removed_or_corrupt_record_is_rewritten_from_the_installed_store_without_traffic() {
    for case in 0..2 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let (path, before) = record(&f);
        match case {
            0 => fs::remove_file(&path).unwrap(),
            _ => fs::write(&path, b"not a trust record").unwrap(),
        }
        assert!(trust(&f).recorded().unwrap().is_none());
        f.run(&u, FROZEN).unwrap();
        // The store's trees and environment are the user's own; only the
        // record binding this repository is written again.
        assert_eq!(u.hits(), 3);
        assert_eq!(record(&f).1, before);
        assert!(trust(&f).recorded().unwrap().is_some());
        f.no_execution();
    }
}

#[test]
fn lock_drift_is_setup_reuse_not_run_authority_and_failed_setup_keeps_record() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let (path, old) = record(&f);
    fs::write(f.owned.root().join("turbo.lock"), format!("{}\n", f.lock)).unwrap();
    // The same tools under other lock bytes: setup reuses every tree, but run
    // consumers are not authorized until setup records the new digest.
    let trust = trust(&f);
    assert!(trust.recorded().unwrap().unwrap().stale);
    assert_ne!(
        old["lock_sha256"],
        format!(
            "{:x}",
            Sha256::digest(fs::read(f.owned.root().join("turbo.lock")).unwrap())
        )
    );
    f.run(&u, FROZEN).unwrap();
    assert_eq!(u.hits(), 3);
    assert_ne!(record(&f).1["lock_sha256"], old["lock_sha256"]);
    assert_eq!(record(&f).1["env"], old["env"]);
    // A forced setup that must restage a damaged tree from a corrupt source
    // fails without touching the record.
    let before = fs::read(&path).unwrap();
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    turborepo_setup::test_support::damage(&current.tree("pnpm").join("dist/resource"), "damaged");
    let broken = Upstream::new(&f, true, None); // Corrupt pnpm bytes.
    assert!(
        f.run(&broken, &["--force", "--tools-only", "--frozen"])
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    f.no_execution();
}

#[test]
fn shell_hook_view_keeps_recorded_tools_on_lock_drift_but_never_repo_state() {
    for case in 0..4 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let bin = Installed::open(&f.owned).current().unwrap().unwrap().bin;
        let fresh = trust(&f).recorded().unwrap().unwrap();
        assert!(fresh.activation.bin() == bin && !fresh.stale);
        match case {
            0 => fs::write(f.owned.root().join("turbo.lock"), format!("{}\n", f.lock)).unwrap(),
            // The editor link is not evidence.
            1 => fs::remove_file(f.owned.root().join(".turbo/bin")).unwrap(),
            2 => fs::remove_dir_all(bin.parent().unwrap()).unwrap(),
            _ => fs::remove_file(record(&f).0).unwrap(),
        }
        let recorded = trust(&f).recorded().unwrap();
        if case < 2 {
            let recorded = recorded.unwrap();
            assert!(recorded.activation.bin() == bin && recorded.stale == (case == 0));
        } else {
            assert!(recorded.is_none(), "case {case}");
        }
        f.no_execution();
    }
}

#[test]
fn moved_and_same_path_recloned_roots_cannot_reuse_records() {
    for same_path in [false, true] {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let moved = f.owned.root().with_file_name("moved-repo");
        fs::rename(f.owned.root(), &moved).unwrap();
        let root = if same_path {
            // Keep the old inode alive, and recreate a clone at its former path.
            fs::create_dir(f.owned.root()).unwrap();
            fs::write(f.owned.root().join("turbo.lock"), f.lock.to_string()).unwrap();
            fs::rename(moved.join(".turbo"), f.owned.root().join(".turbo")).unwrap();
            f.owned.root()
        } else {
            moved.as_path()
        };
        let state = TrustStore::new(&UserStore::at(f.owned.home(), root).unwrap(), root).unwrap();
        assert!(state.recorded().unwrap().is_none());
    }
}

#[test]
fn a_directory_restored_on_another_device_keeps_the_recorded_tools() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let installed = trust(&f).recorded().unwrap().unwrap();
    let (path, mut value) = record(&f);
    // A repository restored, copied or remounted on another device at the same
    // path keeps this path and inode but gets a new device id.
    value["device"] = Value::from(value["device"].as_u64().unwrap() + 1);
    fs::write(&path, value.to_string()).unwrap();
    let recorded = trust(&f).recorded().unwrap().unwrap();
    assert!(recorded.activation.bin() == installed.activation.bin());
    assert!(!recorded.stale);
    assert!(trust(&f).recorded_versions().is_some());
    f.no_execution();
}

#[test]
fn unsafe_external_state_fails_before_traffic_without_following_links() {
    for case in 0..6 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        if case == 0 {
            assert!(UserStore::at(f.owned.root(), f.owned.root()).is_err());
            symlink(f.owned.root(), f.owned.home().join("repos")).unwrap();
        } else {
            f.run(&u, FROZEN).unwrap();
            let (path, _) = record(&f);
            if case == 1 {
                fs::remove_file(&path).unwrap();
                symlink(f.owned.root().join("turbo.lock"), &path).unwrap();
            } else if case == 2 {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
            } else if case == 3 {
                let directory = path.parent().unwrap();
                fs::set_permissions(directory, fs::Permissions::from_mode(0o777)).unwrap();
            } else if case == 4 {
                fs::hard_link(&path, f.owned.root().join("committed-record")).unwrap();
            } else {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(1024 * 1024 + 1)
                    .unwrap();
            }
        }
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        let before = f.manifest();
        assert!(f.run(&empty, FROZEN).is_err());
        assert_eq!(empty.hits(), 0);
        assert_eq!(f.manifest(), before);
        f.no_execution();
    }
}

#[test]
fn existing_store_ancestors_only_need_to_be_unwritable_by_everyone() {
    // Ubuntu's 0002 umask leaves an existing `~/.local/share` 0775.
    let f = Fixture::new();
    let store = f.owned.home();
    let ancestor = store.parent().unwrap();
    fs::set_permissions(ancestor, fs::Permissions::from_mode(0o775)).unwrap();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let (path, _) = record(&f);
    let records = path.parent().unwrap();
    assert_eq!(fs::metadata(records).unwrap().mode() & 0o777, 0o700);
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, FROZEN).unwrap();
    // The record directory stays private, nobody else may write the store,
    // and a world-writable ancestor fails closed before traffic.
    let store_mode = fs::metadata(store).unwrap().mode() & 0o777;
    for (path, mode, restore) in [
        (records, 0o750, 0o700),
        (store, 0o775, store_mode),
        (ancestor, 0o777, 0o775),
    ] {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(f.run(&empty, FROZEN).is_err(), "{path:?} {mode:o}");
        fs::set_permissions(path, fs::Permissions::from_mode(restore)).unwrap();
    }
    f.run(&empty, FROZEN).unwrap();
    assert_eq!(empty.hits(), 0);
    f.no_execution();
}
