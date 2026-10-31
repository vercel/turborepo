use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use turborepo_tool_install::{Store, Tool, UserStore};

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
fn seed(f: &Fixture, git: bool) {
    let lock = f.sources().unwrap();
    let plans = provision::plans(&lock, lock.previous_lock().unwrap()).unwrap();
    let mut tools = vec![
        plans.node.unwrap().inventory_tool().clone(),
        plans.pnpm.unwrap().inventory_tool().clone(),
    ];
    if git {
        tools.push(Tool {
            id: "git".into(),
            version: "1.0.0".into(),
            platform: "any".into(),
            artifact_sha256: "a".repeat(64),
            executables: [("git".into(), "bin/git".into())].into(),
            vars: Default::default(),
        });
    }
    Store::open(f.owned.root())
        .unwrap()
        .force_reconcile_checked(
            &tools,
            |tool, root| {
                for path in tool.tree_files() {
                    let file = root.join(path);
                    fs::create_dir_all(file.parent().unwrap())?;
                    fs::write(&file, "#!/bin/sh\ntouch probe-ran\nexit 91\n")?;
                    fs::set_permissions(&file, fs::Permissions::from_mode(0o755))?;
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
}

#[test]
fn fresh_setup_records_exact_private_external_evidence_and_repeat_is_noop() {
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let (path, value) = record(&f);
    assert!(!path.starts_with(f.owned.root()));
    let root = fs::metadata(f.owned.root()).unwrap();
    let installation = &value["installation"];
    assert_eq!(value["schema"], 1);
    assert_eq!(installation["root"], f.owned.root().to_str().unwrap());
    assert_eq!(installation["device"], root.dev());
    assert_eq!(installation["inode"], root.ino());
    assert_eq!(
        value["lock_sha256"],
        format!(
            "{:x}",
            Sha256::digest(fs::read(f.owned.root().join("turbo.lock")).unwrap())
        )
    );
    assert_eq!(
        installation["manifest_sha256"],
        format!("{:x}", Sha256::digest(f.manifest().unwrap()))
    );
    let current = Store::inspect(f.owned.root()).unwrap().unwrap();
    assert_eq!(
        installation["tools"],
        serde_json::to_value(current.tools).unwrap()
    );
    assert_eq!(installation["bin"], current.bin.to_str().unwrap());
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, FROZEN).unwrap();
    assert_eq!(empty.hits(), 0);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    f.no_execution();
}

#[test]
fn repository_inventory_is_never_promoted_to_trust_or_executed() {
    let f = Fixture::new();
    seed(&f, true);
    assert!(
        Store::inspect(f.owned.root())
            .unwrap()
            .unwrap()
            .bin
            .join("git")
            .exists()
    );
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    let current = Store::inspect(f.owned.root()).unwrap().unwrap();
    assert!(!current.bin.join("git").exists());
    assert!(
        trust(&f)
            .allows_reuse(&Store::open(f.owned.root()).unwrap())
            .unwrap()
    );
    f.no_execution();
}

#[test]
fn coherent_repo_forgery_and_removed_or_corrupt_user_record_require_fresh_verification() {
    for case in 0..4 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let (path, _) = record(&f);
        match case {
            0 => seed(&f, false), // Healthy, self-consistent replacement inventory.
            1 => fs::remove_file(&path).unwrap(),
            2 => fs::write(&path, b"not a trust record").unwrap(),
            _ => {
                let current = Store::inspect(f.owned.root()).unwrap().unwrap();
                fs::remove_file(current.bin.join("node")).unwrap();
            }
        }
        assert!(
            !trust(&f)
                .allows_reuse(&Store::open(f.owned.root()).unwrap())
                .unwrap()
        );
        let manifest = f.manifest().unwrap();
        f.run(&u, FROZEN).unwrap();
        assert_ne!(f.manifest().unwrap(), manifest);
        assert_eq!(u.hits(), 6);
        assert!(
            trust(&f)
                .allows_reuse(&Store::open(f.owned.root()).unwrap())
                .unwrap()
        );
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
    // Exact unchanged tool bytes may be reused by setup, not stale run consumers.
    assert!(
        trust(&f)
            .allows_reuse(&Store::open(f.owned.root()).unwrap())
            .unwrap()
    );
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
    let before = fs::read(&path).unwrap();
    let broken = Upstream::new(&f, true, None);
    assert!(
        f.run(&broken, &["--force", "--tools-only", "--frozen"])
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    f.no_execution();
}

#[test]
fn shell_hook_view_keeps_recorded_tools_on_lock_drift_but_never_repo_inventory() {
    for case in 0..5 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        f.run(&u, FROZEN).unwrap();
        let bin = Store::inspect(f.owned.root()).unwrap().unwrap().bin;
        let fresh = trust(&f).recorded().unwrap().unwrap();
        assert!(fresh.bin == bin && !fresh.stale);
        match case {
            0 => fs::write(f.owned.root().join("turbo.lock"), format!("{}\n", f.lock)).unwrap(),
            1 => seed(&f, false),
            2 => fs::remove_file(bin.join("node")).unwrap(),
            3 => fs::remove_dir_all(f.owned.root().join(".turbo/tools")).unwrap(),
            _ => fs::remove_file(record(&f).0).unwrap(),
        }
        let recorded = trust(&f).recorded().unwrap();
        if case == 0 {
            let recorded = recorded.unwrap();
            assert!(recorded.bin == bin && recorded.stale);
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
        assert!(!state.allows_reuse(&Store::open(root).unwrap()).unwrap());
    }
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

#[test]
fn setup_prunes_only_records_whose_root_is_gone_or_replaced() {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    f.run(&u, FROZEN).unwrap();
    let (own, value) = record(&f);
    let repos = f.owned.home().join("repos");
    let write = |name: String, root: &std::path::Path, identity: (u64, u64)| {
        let mut other = value.clone();
        other["installation"]["root"] = json!(root);
        other["installation"]["device"] = json!(identity.0);
        other["installation"]["inode"] = json!(identity.1);
        let directory = repos.join(name);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        fs::create_dir(directory.join("cargo-home")).unwrap();
        let path = directory.join("record.json");
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap()
            .write_all(&serde_json::to_vec(&other).unwrap())
            .unwrap();
        path
    };
    let named = |root: &std::path::Path| {
        format!("{:x}", Sha256::digest(root.as_os_str().as_encoded_bytes()))
    };
    let live = f.owned.home().join("live-repo");
    let recloned = f.owned.home().join("recloned-repo");
    let deleted = f.owned.home().join("deleted-repo");
    for dir in [&live, &recloned] {
        fs::create_dir(dir).unwrap();
    }
    let identity = |root: &std::path::Path| {
        let metadata = fs::metadata(root).unwrap();
        (metadata.dev(), metadata.ino())
    };
    let kept = write(named(&live), &live, identity(&live));
    let replaced = write(
        named(&recloned),
        &recloned,
        (identity(&recloned).0, u64::MAX),
    );
    let gone = write(named(&deleted), &deleted, identity(&live));
    // A record filed under another root's name never describes that root.
    let misfiled = write("0".repeat(64), &deleted, identity(&live));

    // A repeat setup prunes without traffic and keeps its own authority.
    let empty = LoopbackServer::new([], |_| {}).unwrap();
    f.run(&empty, FROZEN).unwrap();
    assert_eq!(empty.hits(), 0);
    assert!(own.exists() && kept.exists() && misfiled.exists());
    // A pruned repository's whole directory goes, its Cargo home included.
    assert!(!replaced.parent().unwrap().exists() && !gone.parent().unwrap().exists());
    assert!(kept.parent().unwrap().join("cargo-home").is_dir());
    assert!(
        trust(&f)
            .allows_reuse(&Store::open(f.owned.root()).unwrap())
            .unwrap()
    );
    f.no_execution();
}
