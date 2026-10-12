use super::*;

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

#[test]
fn version_change_leaves_one_inactive_generation_that_cleanup_removes() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    assert_eq!(store.inactive_use().unwrap(), DiskUse::default());
    store.reconcile(&[versioned("1.0.0")], populate).unwrap();
    let selected = store.selected_use().unwrap();
    assert_eq!(selected.entries, 1);
    assert!(selected.bytes > 0);
    store.reconcile(&[versioned("2.0.0")], populate).unwrap();
    let inactive = store.inactive_use().unwrap();
    assert_eq!(inactive.entries, 1);
    assert!(inactive.bytes > 0);
    assert_eq!(store.remove_inactive().unwrap(), inactive);
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools, vec![versioned("2.0.0")]);
    let generation = current.bin.parent().unwrap().file_name().unwrap();
    let generation = generation.to_string_lossy().into_owned();
    assert_eq!(
        names(&store),
        [
            generation,
            "manifest.json".into(),
            "transaction.lock".into()
        ]
    );
    assert_eq!(store.remove_inactive().unwrap(), DiskUse::default());
    assert!(Store::inspect(repo.path()).unwrap().is_some());
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
    assert_eq!(store.remove_inactive().unwrap().entries, 2);
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
    let old = store
        .current()
        .unwrap()
        .unwrap()
        .bin
        .parent()
        .unwrap()
        .to_owned();
    store.reconcile(&[versioned("2.0.0")], populate).unwrap();
    // A partially removed generation is never selectable again.
    fs::remove_dir_all(old.join("bin")).unwrap();
    assert!(Store::inspect(repo.path()).unwrap().is_some());
    assert!(store.is_current(&[versioned("2.0.0")]).unwrap());
    assert_eq!(store.remove_inactive().unwrap().entries, 1);
    assert!(!old.exists());
    assert!(store.is_current(&[versioned("2.0.0")]).unwrap());
}

#[test]
fn invalid_selection_fails_closed_without_removing_anything() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[versioned("1.0.0")], populate).unwrap();
    store.reconcile(&[versioned("2.0.0")], populate).unwrap();
    fs::write(store.root.join("manifest.json"), b"{\"schema\":1}").unwrap();
    let before = names(&store);
    assert!(store.inactive_use().is_err());
    assert!(store.remove_inactive().is_err());
    assert_eq!(names(&store), before);
}
