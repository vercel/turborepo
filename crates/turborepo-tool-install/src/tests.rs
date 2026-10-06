#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};

use super::*;

mod recovery;

fn tool(id: &str) -> Tool {
    Tool {
        id: id.into(),
        version: "1.2.3".into(),
        platform: "linux-x86_64-gnu".into(),
        artifact_sha256: "a".repeat(64),
        executables: BTreeMap::from([(id.into(), "bin/tool".into())]),
    }
}

fn populate(tool: &Tool, root: &Path) -> Result<(), Error> {
    fs::create_dir(root.join("bin"))?;
    fs::write(root.join("bin/tool"), &tool.version)?;
    fs::set_permissions(root.join("bin/tool"), fs::Permissions::from_mode(0o755))?;
    fs::write(root.join("resource"), b"adjacent resource")?;
    fs::write(root.join(".resource"), b"hidden adjacent resource")?;
    symlink("tool", root.join("bin/alias"))?;
    Ok(())
}

fn manifest(store: &Store) -> Vec<u8> {
    fs::read(store.root.join("manifest.json")).unwrap()
}

#[test]
fn no_op_replacement_removal_and_full_layout() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let a = tool("node");
    let b = tool("pnpm");
    assert!(store.current().unwrap().is_none());
    assert_eq!(
        store.reconcile(&[b.clone(), a.clone()], populate).unwrap(),
        Outcome::Replaced
    );
    let before = manifest(&store);
    let old_bin = store.current().unwrap().unwrap().bin;
    assert_eq!(
        store
            .reconcile(&[a.clone(), b.clone()], |_, _| panic!(
                "no-op must not stage"
            ))
            .unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(manifest(&store), before);
    assert_eq!(fs::read(old_bin.join("node")).unwrap(), b"1.2.3");
    assert_eq!(
        fs::read(old_bin.join("../tools/node/resource")).unwrap(),
        b"adjacent resource"
    );
    assert_eq!(
        store
            .reconcile(&[a], |_, _| panic!("unchanged tools are reused"))
            .unwrap(),
        Outcome::Replaced
    );
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools.len(), 1);
    assert!(!current.bin.join("pnpm").exists());
    assert!(current.bin.join("node").exists());
    assert_eq!(
        store.reconcile(&[], |_, _| panic!()).unwrap(),
        Outcome::Replaced
    );
    assert_eq!(
        fs::read_dir(store.current().unwrap().unwrap().bin)
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        store.reconcile(&[], |_, _| panic!()).unwrap(),
        Outcome::Unchanged
    );
}

#[test]
fn failed_staging_and_invalid_tree_preserve_prior_bytes() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let mut a = tool("node");
    store.reconcile(&[a.clone()], populate).unwrap();
    let before = manifest(&store);
    let bin = store.current().unwrap().unwrap().bin;
    a.version = "2.0.0".into();
    let failed = store.reconcile(&[a.clone()], |_, root| {
        fs::write(root.join("partial"), b"incomplete")?;
        Err(Error::Io(io::Error::other("interrupted")))
    });
    assert!(failed.is_err());
    assert!(store.reconcile(&[a.clone()], |_, _| Ok(())).is_err());
    assert!(
        store
            .reconcile(&[a], |tool, root| {
                populate(tool, root)?;
                symlink(repo.path(), root.join("escape"))?;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(manifest(&store), before);
    assert_eq!(fs::read(bin.join("node")).unwrap(), b"1.2.3");
    assert_eq!(fs::read_dir(&store.root).unwrap().count(), 3); // lock, manifest, prior generation
}

#[test]
fn exact_identity_and_damage_require_reinstallation() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let mut a = tool("node");
    store.reconcile(&[a.clone()], populate).unwrap();
    for change in 0..4 {
        match change {
            0 => a.artifact_sha256 = "b".repeat(64),
            1 => a.platform = "darwin-arm64".into(),
            2 => a.version = "2.0.0".into(),
            _ => {
                a.executables.insert("alias".into(), "bin/alias".into());
            }
        }
        let mut calls = 0;
        assert_eq!(
            store
                .reconcile(&[a.clone()], |tool, root| {
                    calls += 1;
                    populate(tool, root)
                })
                .unwrap(),
            Outcome::Replaced
        );
        assert_eq!(calls, 1);
    }
    let bin = store.current().unwrap().unwrap().bin;
    fs::write(bin.join("node"), b"tampered").unwrap();
    assert!(store.current().is_err());
    assert_eq!(store.reconcile(&[a], populate).unwrap(), Outcome::Replaced);
    assert_eq!(
        fs::read(store.current().unwrap().unwrap().bin.join("node")).unwrap(),
        b"2.0.0"
    );
}

#[test]
fn untrusted_paths_collisions_and_bookkeeping_cannot_escape() {
    let repo = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), repo.path().join(".turbo")).unwrap();
    assert!(matches!(Store::open(repo.path()), Err(Error::UnsafePath)));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    fs::remove_file(repo.path().join(".turbo")).unwrap();
    fs::create_dir(repo.path().join(".turbo")).unwrap();
    symlink(outside.path(), repo.path().join(".turbo/tools")).unwrap();
    assert!(matches!(Store::open(repo.path()), Err(Error::UnsafePath)));
    fs::remove_file(repo.path().join(".turbo/tools")).unwrap();
    fs::set_permissions(
        repo.path().join(".turbo"),
        fs::Permissions::from_mode(0o777),
    )
    .unwrap();
    assert!(matches!(Store::open(repo.path()), Err(Error::UnsafePath)));
    fs::set_permissions(
        repo.path().join(".turbo"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    for bad in [
        "../escape",
        "/absolute",
        "a/b",
        "Node",
        "con",
        "a:",
        ".",
        "",
    ] {
        let mut a = tool("node");
        a.id = bad.into();
        assert!(store.reconcile(&[a], |_, _| panic!()).is_err());
    }
    let a = tool("node");
    let mut b = tool("pnpm");
    b.executables = a.executables.clone();
    assert!(store.reconcile(&[a.clone(), b], |_, _| panic!()).is_err());
    let mut b = tool("pnpm");
    b.executables.insert("escape".into(), "../escape".into());
    assert!(store.reconcile(&[b], |_, _| panic!()).is_err());
    store.reconcile(&[a], populate).unwrap();
    let before = manifest(&store);
    let mut forged: Inventory = serde_json::from_slice(&before).unwrap();
    forged.generation = "../outside".into();
    fs::write(
        store.root.join("manifest.json"),
        serde_json::to_vec(&forged).unwrap(),
    )
    .unwrap();
    assert!(store.reconcile(&[], |_, _| panic!()).is_err());
    fs::remove_file(store.root.join("manifest.json")).unwrap();
    let victim = outside.path().join("victim");
    fs::write(&victim, &before).unwrap();
    symlink(&victim, store.root.join("manifest.json")).unwrap();
    assert!(store.reconcile(&[], |_, _| panic!()).is_err());
    assert_eq!(fs::read(victim).unwrap(), before);
}

#[test]
fn lock_is_clone_local_and_released_on_drop() {
    let repo = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    assert!(matches!(Store::open(repo.path()), Err(Error::Busy)));
    assert!(Store::open(other.path()).is_ok());
    drop(store);
    assert!(Store::open(repo.path()).is_ok());
}

#[test]
fn abrupt_process_exit_keeps_prior_generation_and_releases_lock() {
    let repo = tempfile::tempdir().unwrap();
    let a = tool("node");
    let before = {
        let mut store = Store::open(repo.path()).unwrap();
        store.reconcile(std::slice::from_ref(&a), populate).unwrap();
        manifest(&store)
    };
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::crash_child", "--ignored"])
        .env("TURBO_INSTALL_CRASH_REPO", repo.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(79));
    let mut store = Store::open(repo.path()).unwrap();
    assert_eq!(manifest(&store), before);
    assert_eq!(
        fs::read(store.current().unwrap().unwrap().bin.join("node")).unwrap(),
        b"1.2.3"
    );
    let mut a = a;
    a.version = "2.0.0".into();
    assert_eq!(store.reconcile(&[a], populate).unwrap(), Outcome::Replaced);
}

#[test]
#[ignore = "subprocess fixture for abrupt interruption without destructors"]
fn crash_child() {
    let repo = std::env::var_os("TURBO_INSTALL_CRASH_REPO").expect("child fixture path");
    let mut store = Store::open(Path::new(&repo)).unwrap();
    let mut a = tool("node");
    a.version = "2.0.0".into();
    store
        .reconcile(&[a], |tool, root| {
            populate(tool, root)?;
            std::process::exit(79);
        })
        .unwrap();
}
