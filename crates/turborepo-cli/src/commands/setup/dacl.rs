//! Windows private-object precondition for setup user state, not execution
//! authorization. A private object is owned by the current user, and every
//! grant in its DACL is to that user. Deny entries are allowed. Grants to any
//! other principal fail, whether explicit, inherited or inherit-only, and so
//! does a null DACL. New objects get a protected owner-only DACL at creation,
//! so the profile's inherited SYSTEM/Administrators grants never apply and an
//! elevated token's default Administrators owner is never used. Reparse points
//! are opened, never followed, and rejected.

use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::null_mut,
};

use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        ACE_HEADER, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, GetTokenInformation,
        OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW,
        FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetFileInformationByHandle,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

const DIRECTORY: u32 = 0x10; // FILE_ATTRIBUTE_DIRECTORY
const REPARSE_POINT: u32 = 0x400; // FILE_ATTRIBUTE_REPARSE_POINT

fn unsafe_descriptor() -> io::Error {
    io::Error::other("unsafe setup user-state security descriptor")
}

/// LocalAlloc'd memory returned by the security APIs.
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: allocated by the API that returned it and freed exactly once.
        unsafe { LocalFree(self.0) };
    }
}

/// The process token's user SID. `u64` storage aligns the TOKEN_USER header.
struct User(Vec<u64>);
impl User {
    fn current() -> io::Result<Self> {
        let mut token = null_mut();
        // SAFETY: pseudo-handle for this process; output is local storage.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a token handle this function now exclusively owns.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut needed = 0;
        // SAFETY: size query without a buffer; the expected failure sets needed.
        unsafe {
            GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut needed)
        };
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: the buffer holds at least `needed` bytes.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buffer))
    }

    fn sid(&self) -> PSID {
        // SAFETY: a filled TOKEN_USER whose SID points into this same buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    /// Protected owner-only descriptor for a new object. `OICI` makes later
    /// children of a directory inherit the same single grant.
    fn descriptor(&self, inherit: &str) -> io::Result<Local> {
        let mut text = null_mut();
        // SAFETY: valid SID; the API allocates the string, which Local frees.
        if unsafe { ConvertSidToStringSidW(self.sid(), &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let text = Local(text.cast());
        let wide = text.0.cast::<u16>();
        // SAFETY: a NUL-terminated wide string returned by the API.
        let sid = String::from_utf16(unsafe {
            std::slice::from_raw_parts(wide, (0..).take_while(|&i| *wide.add(i) != 0).count())
        })
        .map_err(io::Error::other)?;
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;{inherit};FA;;;{sid})\0")
            .encode_utf16()
            .collect();
        let mut descriptor = null_mut();
        // SAFETY: NUL-terminated SDDL; the API allocates the descriptor.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Local(descriptor))
    }
}

fn attributes(descriptor: &Local) -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// Creates a private directory. An existing path is returned as
/// AlreadyExists, never adopted; the caller checks it.
pub(super) fn create_directory(path: &Path) -> io::Result<()> {
    let descriptor = User::current()?.descriptor("OICI")?;
    // SAFETY: NUL-terminated path; the attributes outlive the call.
    if unsafe { CreateDirectoryW(wide(path).as_ptr(), &attributes(&descriptor)) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Creates a new private file. Delete sharing permits atomic replacement
/// while open.
pub(super) fn create_file(path: &Path) -> io::Result<File> {
    let descriptor = User::current()?.descriptor("")?;
    // SAFETY: as above. CREATE_NEW never opens an existing file or link.
    let handle = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes(&descriptor),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a valid handle exclusively owned by the returned File.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Opens the object itself, never a reparse target, and requires the
/// expected type. Checks then apply to this handle, not to the path.
pub(super) fn open(path: &Path, directory: bool) -> io::Result<File> {
    // FILE_FLAG_OPEN_REPARSE_POINT, plus FILE_FLAG_BACKUP_SEMANTICS for
    // directories.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(0x0020_0000 | if directory { 0x0200_0000 } else { 0 })
        .open(path)?;
    let attributes = information(&file)?.dwFileAttributes;
    if attributes & REPARSE_POINT != 0 || (attributes & DIRECTORY != 0) != directory {
        return Err(unsafe_descriptor());
    }
    Ok(file)
}

pub(super) fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    // SAFETY: plain-data output for a live handle.
    let mut information = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(information)
}

/// Private-object policy for a live handle opened with read access.
pub(super) fn check(file: &File) -> io::Result<()> {
    let user = User::current()?;
    let (mut owner, mut dacl, mut descriptor) = (null_mut(), null_mut(), null_mut());
    // SAFETY: live handle; owner and DACL point into the returned descriptor,
    // which stays allocated until `_descriptor` drops.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _descriptor = Local(descriptor);
    // SAFETY: both SIDs are valid for the duration of the call.
    if dacl.is_null() || owner.is_null() || unsafe { EqualSid(owner, user.sid()) } == 0 {
        return Err(unsafe_descriptor());
    }
    // SAFETY: plain-data output for a valid ACL.
    let mut size: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe {
        GetAclInformation(
            dacl,
            (&raw mut size).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    for index in 0..size.AceCount {
        let mut ace = null_mut();
        // SAFETY: index is below the ACL's ACE count.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: every ACE starts with a header. ACCESS_ALLOWED_ACE (type 0)
        // continues with a 4-byte mask and then its SID.
        let header = unsafe { *ace.cast::<ACE_HEADER>() };
        let user_grant = header.AceType == 0
            && header.AceSize >= 16
            && unsafe { EqualSid(ace.cast::<u8>().add(8).cast(), user.sid()) } != 0;
        // Deny entries (type 1) only remove access. Reject every other grant
        // and ACE type, regardless of inheritance flags or another deny.
        if header.AceType != 1 && !user_grant {
            return Err(unsafe_descriptor());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    #![allow(clippy::unwrap_used)]
    use std::process::Command;

    use super::*;

    pub(in crate::commands::setup) fn icacls(path: &Path, args: &[&str]) -> bool {
        let output = Command::new("icacls")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        output.status.success()
    }

    /// Junctions need no symlink privilege.
    pub(in crate::commands::setup) fn junction(link: &Path, target: &Path) {
        let status = Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .args([link, target])
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn private(path: &Path, directory: bool) -> bool {
        check(&open(path, directory).unwrap()).is_ok()
    }

    #[test]
    fn new_objects_are_private_and_never_adopt_existing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("private");
        create_directory(&directory).unwrap();
        assert!(private(&directory, true));
        let record = directory.join("record");
        drop(create_file(&record).unwrap());
        assert!(private(&record, false));
        for result in [
            create_directory(&directory).map(drop),
            create_file(&record).map(drop),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        }
    }

    #[test]
    fn grants_to_other_principals_fail_but_deny_entries_do_not() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        let child = parent.join("child");
        create_directory(&parent).unwrap();
        create_directory(&child).unwrap();
        let record = child.join("record");
        drop(create_file(&record).unwrap());
        // Not `(W)`, which includes READ_CONTROL and SYNCHRONIZE.
        assert!(icacls(&record, &["/deny", "*S-1-1-0:(WD)"]));
        assert!(private(&record, false));
        assert!(icacls(&record, &["/grant", "*S-1-1-0:(R)"]));
        assert!(!private(&record, false));
        // Inherit-only on the parent. Protected children do not inherit it.
        assert!(icacls(&parent, &["/grant", "*S-1-1-0:(OI)(CI)(IO)(R)"]));
        assert!(!private(&parent, true));
        assert!(private(&child, true));
        // The owner is unchanged; only the inherited grant differs.
        assert!(icacls(&child, &["/inheritance:e"]));
        assert!(!private(&child, true));
        // Changing the owner needs elevation, which CI runners have.
        let owned = temp.path().join("owned");
        drop(create_file(&owned).unwrap());
        if icacls(&owned, &["/setowner", "*S-1-5-32-544"]) {
            assert!(!private(&owned, false));
        }
    }

    #[test]
    fn reparse_points_and_type_mismatches_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        create_directory(&target).unwrap();
        let record = target.join("record");
        drop(create_file(&record).unwrap());
        let link = temp.path().join("junction");
        junction(&link, &target);
        assert!(open(&link, true).is_err());
        assert!(open(&target, false).is_err());
        assert!(open(&record, true).is_err());
        // File symlinks need Developer Mode or elevation.
        let symlink = temp.path().join("symlink");
        if std::os::windows::fs::symlink_file(&record, &symlink).is_ok() {
            assert!(open(&symlink, false).is_err());
        }
    }
}
