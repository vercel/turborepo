//! Windows store ownership: nobody but this user, SYSTEM and Administrators
//! may be able to change what a seal vouches for.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{cell::Cell, process::Command};

use super::*;

/// Users may modify, as a shared cache directory under `C:\` grants them.
const USERS_MODIFY: &[&str] = &["/grant", "*S-1-5-32-545:(OI)(CI)(M)"];
const USERS_MODIFY_FILE: &[&str] = &["/grant", "*S-1-5-32-545:(M)"];

fn icacls(path: &Path, args: &[&str]) {
    let output = Command::new("icacls")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

fn tool() -> Tool {
    Tool {
        id: "tool".into(),
        version: "1.2.3".into(),
        platform: "windows-x86_64".into(),
        artifact_sha256: "a".repeat(64),
        executables: BTreeMap::from([("tool".into(), "bin/tool.exe".into())]),
        vars: Default::default(),
        staging: Staging::new("fixture", 1, &()).unwrap(),
    }
}

fn populate(root: &Path) -> Result<(), Error> {
    fs::create_dir(root.join("bin"))?;
    let system = std::env::var_os("SystemRoot").expect("SystemRoot");
    fs::copy(
        Path::new(&system).join("System32").join("whoami.exe"),
        root.join("bin").join("tool.exe"),
    )?;
    fs::write(root.join("resource"), "resource")?;
    Ok(())
}

struct Fixture {
    _home: tempfile::TempDir,
    _outside: tempfile::TempDir,
    repo: tempfile::TempDir,
    store: UserStore,
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = UserStore::at(&home.path().join("store"), outside.path()).unwrap();
    Fixture {
        _home: home,
        _outside: outside,
        repo: tempfile::tempdir().unwrap(),
        store,
    }
}

fn ensure(f: &Fixture, reuse: Reuse, staged: &Cell<usize>) -> Ensured {
    let selection = Selection::new(vec![tool()]).unwrap();
    f.store
        .begin_setup(f.repo.path(), reuse)
        .unwrap()
        .ensure(
            &selection,
            |_, root| {
                staged.set(staged.get() + 1);
                populate(root)
            },
            |_| Ok(()),
        )
        .unwrap()
}

fn canonical_repo(f: &Fixture) -> PathBuf {
    fs::canonicalize(f.repo.path()).unwrap()
}

#[test]
fn a_store_created_under_a_shared_directory_is_private() {
    let shared = tempfile::tempdir().unwrap();
    icacls(shared.path(), USERS_MODIFY);
    let outside = tempfile::tempdir().unwrap();
    let path = shared.path().join("tools").join("store");
    let store = UserStore::at(&path, outside.path()).unwrap();
    for created in [path.parent().unwrap(), store.root()] {
        dacl::check(&dacl::open(created, true).unwrap()).unwrap();
    }
}

#[test]
fn an_existing_store_others_can_write_is_refused() {
    let shared = tempfile::tempdir().unwrap();
    icacls(shared.path(), USERS_MODIFY);
    let path = shared.path().join("store");
    // Inherits the shared directory's grant, as a pre-created cache would.
    fs::create_dir(&path).unwrap();
    let outside = tempfile::tempdir().unwrap();
    assert!(matches!(
        UserStore::at(&path, outside.path()),
        Err(Error::UnsafeStore(_))
    ));
    assert!(matches!(
        UserStore::existing(&path, outside.path()),
        Err(Error::UnsafeStore(_))
    ));
}

#[test]
fn activation_refuses_an_export_another_user_can_write() {
    let f = fixture();
    let ensured = ensure(&f, Reuse::Sealed, &Cell::new(0));
    let activation = f.store.activate(&canonical_repo(&f), &ensured.env).unwrap();
    let output = Command::new(activation.bin().join("tool.exe"))
        .output()
        .unwrap();
    assert!(output.status.success());

    // The export is a hard link, so this grant is on the tree file too.
    icacls(&activation.bin().join("tool.exe"), USERS_MODIFY_FILE);
    assert!(matches!(
        f.store.activate(&canonical_repo(&f), &ensured.env),
        Err(Error::Damaged(_))
    ));
}

#[test]
fn activation_refuses_a_tree_another_user_can_write() {
    let f = fixture();
    let ensured = ensure(&f, Reuse::Sealed, &Cell::new(0));
    let root = f.store.sealed_tree(&tool()).unwrap().unwrap();
    // On the directory alone, so the export's own descriptor is unchanged.
    icacls(&root, USERS_MODIFY_FILE);
    assert!(matches!(
        f.store.activate(&canonical_repo(&f), &ensured.env),
        Err(Error::NotInstalled(_))
    ));
}

#[test]
fn check_and_forced_setup_treat_a_tree_file_others_can_write_as_damage() {
    let f = fixture();
    let staged = Cell::new(0);
    ensure(&f, Reuse::Sealed, &staged);
    assert_eq!(staged.get(), 1);
    let root = f.store.sealed_tree(&tool()).unwrap().unwrap();
    icacls(&root.join("resource"), USERS_MODIFY_FILE);

    let selection = Selection::new(vec![tool()]).unwrap();
    assert!(f.store.check(&selection).is_err());
    ensure(&f, Reuse::Verified, &staged);
    assert_eq!(staged.get(), 2, "forced setup reused the tree");
    f.store.check(&selection).unwrap();
}
