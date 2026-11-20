//! macOS ACL precondition for setup user state, not execution authorization.
//! Private objects accept no ACL entries (including inherited entries). Public
//! ancestors may have read/search or deny entries, but no allow mutation
//! rights.

use std::{ffi::c_void, fs::File, io, os::fd::AsRawFd};

// Darwin sys/acl.h: opaque handles, ACL_TYPE_EXTENDED, indexed entry access.
unsafe extern "C" {
    fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_uint) -> *mut c_void;
    fn acl_valid(acl: *mut c_void) -> libc::c_int;
    fn acl_get_entry(acl: *mut c_void, index: libc::c_int, entry: *mut *mut c_void) -> libc::c_int;
    fn acl_get_tag_type(entry: *mut c_void, tag: *mut libc::c_uint) -> libc::c_int;
    fn acl_get_permset_mask_np(entry: *mut c_void, mask: *mut u64) -> libc::c_int;
    fn acl_free(acl: *mut c_void) -> libc::c_int;
}

struct Acl(*mut c_void);
impl Drop for Acl {
    fn drop(&mut self) {
        // SAFETY: this is the allocation returned by acl_get_fd_np, freed once.
        unsafe { acl_free(self.0) };
    }
}

pub(super) fn check(file: &File, private: bool) -> io::Result<()> {
    // SAFETY: the descriptor is live; Darwin allocates an independent ACL copy.
    let pointer = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
    if pointer.is_null() {
        let error = io::Error::last_os_error();
        // Darwin reports an absent extended ACL as ENOENT on a live descriptor.
        // Confirm the descriptor remains valid; every other query error fails closed.
        return if error.raw_os_error() == Some(libc::ENOENT) {
            file.metadata().map(|_| ())
        } else {
            Err(error)
        };
    }
    let acl = Acl(pointer);
    // SAFETY: a non-null ACL allocation remains live through all entry reads.
    if unsafe { acl_valid(acl.0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // WRITE_DATA/ADD_FILE, DELETE, APPEND/ADD_SUBDIRECTORY, DELETE_CHILD,
    // WRITE_ATTRIBUTES, WRITE_EXTATTRIBUTES, WRITE_SECURITY and CHANGE_OWNER.
    const MUTATION: u64 =
        (1 << 2) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 8) | (1 << 10) | (1 << 12) | (1 << 13);
    for index in 0..=128 {
        // ACL_MAX_ENTRIES in Darwin sys/acl.h.
        let mut entry = std::ptr::null_mut();
        // Darwin returns 0 for an entry and EINVAL at the end of a valid ACL.
        // SAFETY: ACL was validated; output points to local pointer storage.
        if unsafe { acl_get_entry(acl.0, index, &mut entry) } != 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::EINVAL) {
                Ok(())
            } else {
                Err(error)
            };
        }
        let (mut tag, mut mask) = (0, 0);
        // SAFETY: entry belongs to the live ACL; output storage is correctly typed.
        if unsafe { acl_get_tag_type(entry, &mut tag) } != 0
            || unsafe { acl_get_permset_mask_np(entry, &mut mask) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        // Reject grants even when another ACE denies them, or they are currently
        // inherit-only. Do not attempt to resolve principals or effective order.
        if private || !matches!(tag, 1 | 2) || (tag == 1 && mask & MUTATION != 0) {
            return Err(io::Error::other(
                "unsupported or unsafe setup user-state ACL",
            ));
        }
    }
    Err(io::Error::other(
        "setup user-state ACL exceeds the native entry limit",
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
        process::Command,
    };

    use super::*;

    pub(super) fn add(path: &Path, entry: &str) {
        let output = Command::new("/bin/chmod")
            .args(["+a", entry])
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    #[test]
    fn native_acl_policy_and_inheritance_do_not_rely_on_mode_bits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private");
        fs::write(&path, "record").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let file = File::open(&path).unwrap();
        assert!(check(&file, true).is_ok());
        let before = file.metadata().unwrap();
        add(&path, "group:everyone allow write");
        let after = file.metadata().unwrap();
        assert_eq!(
            (before.uid(), before.ino(), before.mode(), before.nlink()),
            (after.uid(), after.ino(), after.mode(), after.nlink())
        );
        assert!(check(&file, true).is_err());
        assert!(check(&file, false).is_err());
        add(directory.path(), "group:everyone deny delete");
        assert!(check(&File::open(directory.path()).unwrap(), false).is_ok());
        add(
            directory.path(),
            "group:everyone allow read,search,file_inherit,directory_inherit,only_inherit",
        );
        let child = directory.path().join("child");
        fs::DirBuilder::new().create(&child).unwrap();
        fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(check(&File::open(&child).unwrap(), true).is_err());
        let staged = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        assert_eq!(staged.as_file().metadata().unwrap().mode() & 0o777, 0o600);
        assert!(check(staged.as_file(), true).is_err());
    }
}
