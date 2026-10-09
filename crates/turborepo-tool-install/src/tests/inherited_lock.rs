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
