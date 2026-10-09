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
fn fork_copy_drop_cannot_unlock_surviving_parent() {
    let root = repository(true);
    let mut owner = WriterStorage::acquire(root.path()).unwrap();
    let retained_root = owner.root.file.try_clone().unwrap();
    let retained_writer = owner._guard.try_clone().unwrap();
    let closed = [owner.root.file.as_raw_fd(), owner._guard.as_raw_fd()];
    let address = std::ptr::addr_of_mut!(owner) as usize;
    let (root_probe, writer_probe) = probes(root.path());
    // SAFETY: Unix WriterStorage contains only File handles, no heap fields.
    // The fork copy alone is destroyed, allocation-free, before acknowledgement.
    let mut child = unsafe {
        InheritedChild::spawn_with(
            &[retained_root.as_raw_fd(), retained_writer.as_raw_fd()],
            move || {
                std::ptr::drop_in_place(address as *mut WriterStorage);
                for fd in closed {
                    if libc::fcntl(fd, libc::F_GETFD) != -1 {
                        return Err(io::Error::from_raw_os_error(libc::EIO));
                    }
                }
                Ok(())
            },
        )
    };
    child.assert_open();
    drop((retained_root, retained_writer));
    let root_result = root_probe.try_lock();
    let writer_result = writer_probe.try_lock();
    assert!(
        matches!(root_result, Err(fs::TryLockError::WouldBlock))
            && matches!(writer_result, Err(fs::TryLockError::WouldBlock)),
        "fork copy released parent's locks: root={root_result:?}, writer={writer_result:?}"
    );
    drop(owner);
    child.assert_open();
    released(&root_probe);
    released(&writer_probe);
    let next = WriterStorage::acquire(root.path()).unwrap();
    child.assert_open();
    drop(child);
    excluded(&root_probe);
    excluded(&writer_probe);
    drop(next);
}

// Exercise the actual acquisition cleanup guards, not an extra lock wrapper.
fn fork_copy_of_failed_acquisition(stop_after: usize) {
    let root = repository(true);
    drop(WriterStorage::acquire(root.path()).unwrap());
    let (root_probe, writer_probe) = probes(root.path());
    let mut descriptors = Vec::new();
    let mut addresses = Vec::new();
    let mut child = None;
    let error = WriterStorage::acquire_with(root.path(), |lock| {
        descriptors.push(lock.file.unwrap().as_raw_fd());
        addresses.push(lock as *mut AcquiredLock<'_> as usize);
        if descriptors.len() != stop_after {
            return Ok(());
        }
        let copied_addresses = addresses.clone();
        // SAFETY: acquired guards borrow File handles and own no heap fields.
        // Addresses stay in place until this callback returns in the parent;
        // the child only drops copies in writer-before-root order.
        child = Some(unsafe {
            InheritedChild::spawn_with(&descriptors, move || {
                for address in copied_addresses.iter().rev() {
                    std::ptr::drop_in_place(*address as *mut AcquiredLock<'_>);
                }
                Ok(())
            })
        });
        child.as_mut().unwrap().assert_open();
        let root_result = root_probe.try_lock();
        assert!(
            matches!(root_result, Err(fs::TryLockError::WouldBlock)),
            "fork cleanup released parent's root: {root_result:?}"
        );
        if stop_after == 2 {
            excluded(&writer_probe);
        }
        Err(io::Error::from_raw_os_error(libc::EIO))
    })
    .err()
    .unwrap();
    assert_eq!(error.raw_os_error(), Some(libc::EIO));
    child.as_mut().unwrap().assert_open();
    released(&root_probe);
    released(&writer_probe);
}

#[test]
fn fork_copy_of_failed_acquisition_root() {
    fork_copy_of_failed_acquisition(1);
}

#[test]
fn fork_copy_of_failed_acquisition_writer() {
    fork_copy_of_failed_acquisition(2);
}

#[test]
fn fork_copy_transfer_preserves_acquiring_pid() {
    let root = repository(true);
    let file = File::open(root.path()).unwrap();
    let mut guard = AcquiredLock::acquire(&file).unwrap();
    let acquiring_pid = std::process::id();
    let address = std::ptr::addr_of_mut!(guard) as usize;
    let probe = File::open(root.path()).unwrap();
    // SAFETY: ptr::read moves only the child's copy of a heap-free borrowed
    // guard; transfer disarms it. Parent storage stays valid until acknowledgement.
    let mut child = unsafe {
        InheritedChild::spawn_with(&[file.as_raw_fd()], move || {
            let copied = std::ptr::read(address as *const AcquiredLock<'_>);
            if copied.transfer() != acquiring_pid {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            Ok(())
        })
    };
    child.assert_open();
    excluded(&probe);
    drop(guard);
    child.assert_open();
    released(&probe);
}

#[test]
fn failed_ignore_check_releases_inherited_root_lock() {
    let root = repository(false);
    let mut child = None;
    let error = WriterStorage::acquire_with(root.path(), |lock| {
        let file = lock.file.unwrap();
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
    let error = WriterStorage::acquire_with(root.path(), |lock| {
        let file = lock.file.unwrap();
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
        let error = AcquiredLock::from_result(
            &shared,
            Err(io::Error::other("lock failed")),
            std::process::id(),
        )
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
