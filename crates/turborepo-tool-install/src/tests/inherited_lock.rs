use std::os::unix::{fs::MetadataExt, io::AsRawFd};

use super::{inherited_child::InheritedChild, *};

#[test]
fn owner_drop_releases_inherited_lock_before_child_exec() {
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    let path = store.root.join("transaction.lock");
    let inode = fs::metadata(&path).unwrap().ino();
    let mut child = InheritedChild::spawn(&[store._lock.as_raw_fd()]);
    for _ in 0..2 {
        // Failed acquisition must neither unlock nor disturb the active owner.
        assert!(matches!(Store::open(repo.path()), Err(Error::Busy)));
    }
    drop(store);
    child.assert_open();
    let next = Store::open(repo.path()).expect("owner Drop must release before child exec");
    child.assert_open();
    assert!(matches!(Store::open(repo.path()), Err(Error::Busy)));
    drop(child);
    assert!(matches!(Store::open(repo.path()), Err(Error::Busy)));
    drop(next);
    assert!(Store::open(repo.path()).is_ok());
    assert_eq!(fs::metadata(path).unwrap().ino(), inode);
}

#[test]
fn fork_copy_drop_cannot_unlock_surviving_parent() {
    let repo = tempfile::tempdir().unwrap();
    let mut owner = Store::open(repo.path()).unwrap();
    // Keep the heap-owned pathname in the parent; child Store teardown must
    // only run its lock destructor and close File, never invoke the allocator.
    let _root = std::mem::take(&mut owner.root);
    assert_eq!(owner.root.capacity(), 0);
    let retained = owner._lock.try_clone().unwrap();
    let original_fd = owner._lock.as_raw_fd();
    let address = std::ptr::addr_of_mut!(owner) as usize;
    // SAFETY: owner stays in place until acknowledgement. Its only heap field
    // is empty; child teardown uses File drop/unlock and fcntl, then pauses.
    let mut child = unsafe {
        InheritedChild::spawn_with(&[retained.as_raw_fd()], move || {
            std::ptr::drop_in_place(address as *mut Store);
            if libc::fcntl(original_fd, libc::F_GETFD) != -1 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            Ok(())
        })
    };
    child.assert_open();
    drop(retained);
    assert!(
        matches!(Store::open(repo.path()), Err(Error::Busy)),
        "fork copy released surviving parent's Store lock"
    );
    drop(owner);
    child.assert_open();
    let next = Store::open(repo.path()).unwrap();
    child.assert_open();
    drop(child);
    assert!(matches!(Store::open(repo.path()), Err(Error::Busy)));
    drop(next);
}
