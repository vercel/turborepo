use std::{fs, sync::mpsc, thread, time::Duration};

use super::*;

fn repository(ignored: bool) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(root.path())
            .status()
            .unwrap()
            .success()
    );
    if ignored {
        fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
    }
    root
}

fn state(root: &Path, name: &str) -> std::path::PathBuf {
    root.join(".turbo/setup-lock").join(name)
}

#[test]
fn missing_or_overridden_ignore_reports_before_creating_state() {
    let root = repository(false);
    let error = WriterStorage::acquire(root.path()).err().unwrap();
    assert!(error.to_string().contains("explicitly add /.turbo/"));
    assert!(!root.path().join(".gitignore").exists());
    assert!(!root.path().join(".turbo").exists());
    fs::write(
        root.path().join(".gitignore"),
        "/.turbo/*\n!/.turbo/setup-lock/\n",
    )
    .unwrap();
    assert!(WriterStorage::acquire(root.path()).is_err());
    assert!(!root.path().join(".turbo").exists());
    let non_git = tempfile::tempdir().unwrap();
    fs::write(non_git.path().join(".gitignore"), "/.turbo/\n").unwrap();
    assert!(WriterStorage::acquire(non_git.path()).is_err());
    assert!(!non_git.path().join(".turbo").exists());
}

#[test]
fn tracked_bookkeeping_is_rejected_without_recovery_or_mutation() {
    for name in [WRITER, STAGED] {
        let root = repository(true);
        fs::create_dir_all(root.path().join(".turbo/setup-lock")).unwrap();
        let path = state(root.path(), name);
        fs::write(&path, "tracked user state").unwrap();
        assert!(
            Command::new("git")
                .current_dir(root.path())
                .args(["add", "--force", "--"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        assert!(WriterStorage::acquire(root.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"tracked user state");
        assert!(!state(root.path(), if name == WRITER { STAGED } else { WRITER }).exists());
    }
}

#[test]
fn ignored_storage_keeps_root_clean_and_recovers_only_owned_stage() {
    let root = repository(true);
    let ignore = fs::read(root.path().join(".gitignore")).unwrap();
    let mut storage = WriterStorage::acquire(root.path()).unwrap();
    assert!(storage.read_lock().unwrap().is_none());
    storage.replace(b"complete lock\n").unwrap();
    assert_eq!(storage.read_lock().unwrap().unwrap(), b"complete lock\n");
    let writer = fs::metadata(state(root.path(), WRITER)).unwrap();
    // Model a process dying after its stage was created; it cannot be promoted
    // or reclaimed by another writer until this guard is released.
    fs::write(state(root.path(), STAGED), "partial crash bytes").unwrap();
    fs::write(state(root.path(), "unowned"), "retain").unwrap();
    drop(storage);
    let recovered = WriterStorage::acquire(root.path()).unwrap();
    assert!(!state(root.path(), STAGED).exists());
    assert_eq!(recovered.read_lock().unwrap().unwrap(), b"complete lock\n");
    assert_eq!(fs::read(state(root.path(), "unowned")).unwrap(), b"retain");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            writer.ino(),
            fs::metadata(state(root.path(), WRITER)).unwrap().ino()
        );
    }
    #[cfg(not(unix))]
    let _ = writer;
    assert_eq!(fs::read(root.path().join(".gitignore")).unwrap(), ignore);
    let output = Command::new("git")
        .current_dir(root.path())
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "?? .gitignore\n?? turbo.lock\n"
    );
    assert!(!root.path().join(".turbo-lock.writer").exists());
    assert!(!root.path().join("staged").exists());
}

#[test]
fn failed_staging_and_promotion_leave_previous_bytes() {
    let root = repository(true);
    let mut storage = WriterStorage::acquire(root.path()).unwrap();
    storage.replace(b"previous").unwrap();
    assert!(storage.replace(&vec![b'x'; MAX_BYTES + 1]).is_err());
    assert_eq!(storage.read_lock().unwrap().unwrap(), b"previous");
    for promotion in [false, true] {
        assert!(
            storage
                .replace_with(b"candidate", || {
                    assert_eq!(fs::read(root.path().join(TARGET))?, b"previous");
                    assert_eq!(fs::read(state(root.path(), STAGED))?, b"candidate");
                    if promotion {
                        fs::remove_file(state(root.path(), STAGED))
                    } else {
                        Err(io::Error::other("injected failure"))
                    }
                })
                .is_err()
        );
        assert_eq!(storage.read_lock().unwrap().unwrap(), b"previous");
        assert!(!state(root.path(), STAGED).exists());
    }
    storage.replace(b"recovered").unwrap();
    assert_eq!(storage.read_lock().unwrap().unwrap(), b"recovered");
}

#[test]
fn recovery_and_replacement_wait_for_the_same_persistent_inode() {
    let root = repository(true);
    let mut first = WriterStorage::acquire(root.path()).unwrap();
    first.replace(b"previous").unwrap();
    fs::write(state(root.path(), STAGED), "crashed stage").unwrap();
    let path = root.path().to_owned();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let mut guard = WriterStorage::acquire(&path).unwrap();
        assert_eq!(guard.read_lock().unwrap().unwrap(), b"previous");
        guard.replace(b"next").unwrap();
        done_tx.send(()).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(
        fs::read(state(root.path(), STAGED)).unwrap(),
        b"crashed stage"
    );
    drop(first);
    second.join().unwrap();
    assert_eq!(fs::read(root.path().join(TARGET)).unwrap(), b"next");
    assert!(!state(root.path(), STAGED).exists());
    assert!(state(root.path(), WRITER).exists());
}

#[test]
fn non_regular_files_are_never_recovered_or_overwritten() {
    for name in [
        ".turbo",
        ".turbo/setup-lock",
        ".turbo/setup-lock/writer",
        ".turbo/setup-lock/staged",
    ] {
        let root = repository(true);
        let path = root.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if name.ends_with("writer") || name.ends_with("staged") {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, "not a directory").unwrap();
        }
        assert!(WriterStorage::acquire(root.path()).is_err());
        assert!(path.exists());
    }
    let root = repository(true);
    fs::create_dir(root.path().join(TARGET)).unwrap();
    let mut storage = WriterStorage::acquire(root.path()).unwrap();
    assert!(storage.read_lock().is_err());
    assert!(storage.replace(b"candidate").is_err());
    assert!(root.path().join(TARGET).is_dir());
    assert!(!state(root.path(), STAGED).exists());
}

#[cfg(unix)]
#[test]
fn directory_file_and_dangling_symlinks_never_escape_storage() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file"), "untouched").unwrap();
    for name in [
        ".turbo",
        ".turbo/setup-lock",
        ".turbo/setup-lock/writer",
        ".turbo/setup-lock/staged",
    ] {
        for target in [
            outside.path().to_owned(),
            outside.path().join("file"),
            outside.path().join("missing"),
        ] {
            let root = repository(true);
            let path = root.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(&target, &path).unwrap();
            assert!(WriterStorage::acquire(root.path()).is_err());
            assert!(fs::symlink_metadata(&path).unwrap().is_symlink());
            assert_eq!(fs::read(outside.path().join("file")).unwrap(), b"untouched");
            assert!(!outside.path().join("setup-lock").exists());
            assert!(!outside.path().join("missing").exists());
        }
    }
    for target in [outside.path().join("file"), outside.path().join("missing")] {
        let root = repository(true);
        symlink(target, root.path().join(TARGET)).unwrap();
        let mut storage = WriterStorage::acquire(root.path()).unwrap();
        assert!(storage.replace(b"candidate").is_err());
        assert!(storage.read_lock().is_err());
        assert_eq!(fs::read(outside.path().join("file")).unwrap(), b"untouched");
    }
}

#[cfg(unix)]
#[test]
fn recreating_the_cache_cannot_split_active_writer_transactions() {
    let root = repository(true);
    let mut first = WriterStorage::acquire(root.path()).unwrap();
    first.replace(b"previous").unwrap();
    fs::rename(root.path().join(".turbo"), root.path().join("old-cache")).unwrap();
    fs::create_dir(root.path().join(".turbo")).unwrap();
    let path = root.path().to_owned();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let mut guard = WriterStorage::acquire(&path).unwrap();
        assert_eq!(guard.read_lock().unwrap().unwrap(), b"first");
        guard.replace(b"second").unwrap();
        done_tx.send(()).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert!(!state(root.path(), WRITER).exists());
    first.replace(b"first").unwrap();
    drop(first);
    second.join().unwrap();
    assert_eq!(fs::read(root.path().join(TARGET)).unwrap(), b"second");
}

#[cfg(unix)]
#[test]
fn pinned_directory_handles_do_not_follow_a_replaced_cache_path() {
    use std::os::unix::fs::symlink;
    let root = repository(true);
    let outside = tempfile::tempdir().unwrap();
    let mut storage = WriterStorage::acquire(root.path()).unwrap();
    fs::rename(root.path().join(".turbo"), root.path().join("saved-cache")).unwrap();
    symlink(outside.path(), root.path().join(".turbo")).unwrap();
    storage.replace(b"complete").unwrap();
    assert_eq!(fs::read(root.path().join(TARGET)).unwrap(), b"complete");
    assert!(!outside.path().join("setup-lock").exists());
    assert!(!root.path().join("saved-cache/setup-lock/staged").exists());
}
