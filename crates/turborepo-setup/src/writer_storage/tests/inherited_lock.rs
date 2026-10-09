use std::os::unix::{fs::MetadataExt, io::AsRawFd};

use super::{inherited_child::InheritedChild, *};

fn probes(root: &Path) -> (File, File) {
    (
        File::open(root).unwrap(),
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(state(root, WRITER))
            .unwrap(),
    )
}

fn excluded(file: &File) {
    assert!(matches!(file.try_lock(), Err(fs::TryLockError::WouldBlock)));
}

fn released(file: &File) {
    file.try_lock()
        .expect("owner must release before child exec");
    file.unlock().unwrap();
}

#[test]
fn owner_drop_releases_inherited_locks_before_child_exec() {
    let root = repository(true);
    let owner = WriterStorage::acquire(root.path()).unwrap();
    let inode = fs::metadata(state(root.path(), WRITER)).unwrap().ino();
    let mut child = InheritedChild::spawn(&[owner.root.file.as_raw_fd(), owner._guard.as_raw_fd()]);
    let (root_probe, writer_probe) = probes(root.path());
    excluded(&root_probe);
    excluded(&writer_probe);
    drop(owner);
    child.assert_open();
    released(&root_probe);
    released(&writer_probe);
    let next = WriterStorage::acquire(root.path()).unwrap();
    child.assert_open();
    excluded(&root_probe);
    excluded(&writer_probe);
    drop(child);
    excluded(&root_probe);
    excluded(&writer_probe);
    drop(next);
    assert_eq!(
        fs::metadata(state(root.path(), WRITER)).unwrap().ino(),
        inode
    );
}

#[test]
fn failed_ignore_check_releases_inherited_root_lock() {
    let root = repository(false);
    let mut child = None;
    let error = WriterStorage::acquire_with(root.path(), |file| {
        child = Some(InheritedChild::spawn(&[file.as_raw_fd()]));
        Ok(())
    })
    .err()
    .unwrap();
    assert!(error.to_string().contains("explicitly add /.turbo/"));
    child.as_mut().unwrap().assert_open();
    assert!(!root.path().join(".turbo").exists());
    released(&File::open(root.path()).unwrap());
    fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
    let next = WriterStorage::acquire(root.path()).unwrap();
    drop(child);
    let (root_probe, writer_probe) = probes(root.path());
    excluded(&root_probe);
    excluded(&writer_probe);
    drop(next);
}

#[test]
fn failed_recovery_releases_both_inherited_locks() {
    let root = repository(true);
    let mut descriptors = Vec::new();
    let mut child = None;
    let error = WriterStorage::acquire_with(root.path(), |file| {
        descriptors.push(file.as_raw_fd());
        if descriptors.len() == 2 {
            child = Some(InheritedChild::spawn(&descriptors));
            fs::create_dir(state(root.path(), STAGED))?;
        }
        Ok(())
    })
    .err()
    .unwrap();
    assert!(
        error
            .to_string()
            .contains("invalid setup storage file type")
    );
    child.as_mut().unwrap().assert_open();
    let inode = fs::metadata(state(root.path(), WRITER)).unwrap().ino();
    let (root_probe, writer_probe) = probes(root.path());
    released(&root_probe);
    released(&writer_probe);
    fs::remove_dir(state(root.path(), STAGED)).unwrap();
    let next = WriterStorage::acquire(root.path()).unwrap();
    drop(child);
    excluded(&root_probe);
    excluded(&writer_probe);
    drop(next);
    assert_eq!(
        fs::metadata(state(root.path(), WRITER)).unwrap().ino(),
        inode
    );
}

#[test]
fn failed_acquisition_never_unlocks_shared_owner_descriptors() {
    let root = repository(true);
    let owner = WriterStorage::acquire(root.path()).unwrap();
    let (root_probe, writer_probe) = probes(root.path());
    for file in [&owner.root.file, &owner._guard] {
        // Inject a failed syscall result on the same open-file description.
        // Err must never construct cleanup that unlocks someone else's owner.
        let shared = file.try_clone().unwrap();
        let error = AcquiredLock::from_result(&shared, Err(io::Error::other("lock failed")))
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "lock failed");
        excluded(&root_probe);
        excluded(&writer_probe);
    }
    drop(owner);
    released(&root_probe);
    released(&writer_probe);
}

#[test]
fn abrupt_exit_recovers_stage_without_owner_drop() {
    let root = repository(true);
    {
        let mut owner = WriterStorage::acquire(root.path()).unwrap();
        owner.replace(b"previous").unwrap();
    }
    let inode = fs::metadata(state(root.path(), WRITER)).unwrap().ino();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "writer_storage::tests::inherited_lock::crash_child",
            "--ignored",
        ])
        .env("TURBO_WRITER_CRASH_ROOT", root.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(79));
    let owner = WriterStorage::acquire(root.path()).unwrap();
    assert_eq!(owner.read_lock().unwrap().unwrap(), b"previous");
    assert!(!state(root.path(), STAGED).exists());
    assert_eq!(fs::read(state(root.path(), "unowned")).unwrap(), b"retain");
    assert_eq!(
        fs::metadata(state(root.path(), WRITER)).unwrap().ino(),
        inode
    );
}

#[test]
#[ignore = "subprocess fixture for abrupt interruption without destructors"]
fn crash_child() {
    let root = std::env::var_os("TURBO_WRITER_CRASH_ROOT").unwrap();
    let _owner = WriterStorage::acquire(Path::new(&root)).unwrap();
    fs::write(state(Path::new(&root), STAGED), b"partial").unwrap();
    fs::write(state(Path::new(&root), "unowned"), b"retain").unwrap();
    std::process::exit(79);
}
