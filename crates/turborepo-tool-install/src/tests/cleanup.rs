use super::*;

const GRACE: Duration = Duration::from_secs(3600);

fn versioned(version: &str) -> Tool {
    Tool {
        version: version.into(),
        ..tool("node")
    }
}

fn names(store: &Store) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(&store.root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn selected(store: &Store) -> PathBuf {
    store
        .current()
        .unwrap()
        .unwrap()
        .bin
        .parent()
        .unwrap()
        .into()
}

fn age(path: &Path, by: Duration) {
    let at = SystemTime::now() - by;
    File::open(path).unwrap().set_modified(at).unwrap();
}

#[test]
fn replaced_generation_is_kept_through_its_grace_period_then_removed() {
    let repo = tempfile::tempdir().unwrap();
    assert_eq!(Store::storage_use(repo.path(), GRACE).unwrap(), None);
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[versioned("1.0.0")], populate).unwrap();
    let old = selected(&store);
    // However long it was selected, the grace starts at replacement.
    age(&old, GRACE * 2);
    store.reconcile(&[versioned("2.0.0")], populate).unwrap();
    let kept = store.remove_inactive(GRACE).unwrap();
    assert_eq!(kept.removable, DiskUse::default());
    assert_eq!(kept.retained.entries, 1);
    assert!(kept.selected.bytes > 0 && old.exists());
    // Read-only inspection agrees and changes nothing.
    let before = names(&store);
    assert_eq!(Store::storage_use(repo.path(), GRACE).unwrap(), Some(kept));
    assert_eq!(names(&store), before);
    age(&old, GRACE);
    let inspected = Store::storage_use(repo.path(), GRACE).unwrap().unwrap();
    assert_eq!(inspected.removable, kept.retained);
    assert!(old.exists());
    assert_eq!(store.remove_inactive(GRACE).unwrap(), inspected);
    let generation = selected(&store)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        names(&store),
        [
            generation,
            "manifest.json".into(),
            "transaction.lock".into()
        ]
    );
    let after = store.remove_inactive(Duration::ZERO).unwrap();
    assert_eq!(after.removable, DiskUse::default());
    assert_eq!(after.selected, kept.selected);
    assert!(store.is_current(&[versioned("2.0.0")]).unwrap());
}

#[test]
fn cleanup_removes_interrupted_staging_but_never_foreign_entries_or_links() {
    let repo = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[tool("node")], populate).unwrap();
    // Crash leftovers: kept staging and an unpersisted manifest candidate.
    fs::create_dir_all(store.root.join("generation-crashed/tools")).unwrap();
    fs::write(store.root.join(".tmpManifest"), b"{}").unwrap();
    // Nothing turbo did not create here is removed, even with owned names.
    fs::write(store.root.join("notes.txt"), b"user file").unwrap();
    fs::write(outside.path().join("keep"), b"outside").unwrap();
    symlink(outside.path(), store.root.join("generation-link")).unwrap();
    fs::write(store.root.join("generation-file"), b"not a generation").unwrap();
    let removed = store.remove_inactive(Duration::ZERO).unwrap();
    assert_eq!(removed.removable.entries, 2);
    assert!(fs::read(outside.path().join("keep")).is_ok());
    for name in ["notes.txt", "generation-link", "generation-file"] {
        assert!(
            fs::symlink_metadata(store.root.join(name)).is_ok(),
            "{name}"
        );
    }
    assert!(!store.root.join("generation-crashed").exists());
    assert!(!store.root.join(".tmpManifest").exists());
    assert_eq!(store.current().unwrap().unwrap().tools, vec![tool("node")]);
}

#[test]
fn interrupted_cleanup_leaves_only_inactive_remnants_for_the_next_run() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[versioned("1.0.0")], populate).unwrap();
    let old = selected(&store);
    store.reconcile(&[versioned("2.0.0")], populate).unwrap();
    // A partially removed generation is never selectable again.
    fs::remove_dir_all(old.join("bin")).unwrap();
    assert!(Store::inspect(repo.path()).unwrap().is_some());
    assert!(store.is_current(&[versioned("2.0.0")]).unwrap());
    age(&old, GRACE);
    assert_eq!(store.remove_inactive(GRACE).unwrap().removable.entries, 1);
    assert!(!old.exists());
    assert!(store.is_current(&[versioned("2.0.0")]).unwrap());
}

#[test]
fn inspection_accepts_shared_parents_but_rejects_an_unowned_store() {
    let repo = tempfile::tempdir().unwrap();
    fs::set_permissions(repo.path(), fs::Permissions::from_mode(0o775)).unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    fs::set_permissions(
        repo.path().join(".turbo"),
        fs::Permissions::from_mode(0o775),
    )
    .unwrap();
    store.reconcile(&[versioned("1.0.0")], populate).unwrap();
    let before = names(&store);
    let usage = store.remove_inactive(GRACE).unwrap();
    assert_eq!(Store::storage_use(repo.path(), GRACE).unwrap(), Some(usage));
    assert_eq!(names(&store), before);
    // Matching Store::open's shared-parent policy must not relax the owned
    // installation boundary or change permissions during read-only reporting.
    fs::set_permissions(&store.root, fs::Permissions::from_mode(0o775)).unwrap();
    assert!(matches!(
        Store::storage_use(repo.path(), GRACE),
        Err(Error::UnsafePath)
    ));
    assert!(matches!(
        store.remove_inactive(GRACE),
        Err(Error::UnsafePath)
    ));
    assert_eq!(names(&store), before);
    assert_eq!(
        fs::metadata(&store.root).unwrap().permissions().mode() & 0o777,
        0o775
    );
}

#[test]
fn invalid_selection_fails_closed_without_removing_anything() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[versioned("1.0.0")], populate).unwrap();
    store.reconcile(&[versioned("2.0.0")], populate).unwrap();
    fs::write(store.root.join("manifest.json"), b"{\"schema\":1}").unwrap();
    let before = names(&store);
    assert!(Store::storage_use(repo.path(), Duration::ZERO).is_err());
    assert!(store.remove_inactive(Duration::ZERO).is_err());
    assert_eq!(names(&store), before);
}
