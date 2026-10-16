//! Windows stat stamps for `Verification::Stamped`, read through
//! attribute-only handles. Only NTFS and ReFS volumes stamp: both give every
//! file a stable ID and advance its change time on every write, including a
//! write through another hard link such as a `bin` export. Stamps come from
//! the file's own record, never from directory listings, whose copies of
//! times and sizes a write through another link leaves stale.

use std::{
    fs::{File, OpenOptions},
    io,
    mem::MaybeUninit,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_INFO_BY_HANDLE_CLASS, FILE_READ_ATTRIBUTES, FILE_STANDARD_INFO, FileBasicInfo, FileIdInfo,
    FileStandardInfo, GetFileInformationByHandleEx, GetVolumeInformationByHandleW,
};

/// `path` itself, never a link target, opened for attributes only, as
/// `fs::symlink_metadata` opens it. Without data access the handle cannot
/// conflict with other handles' sharing modes or wait for an antivirus
/// content scan.
pub(crate) fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// Whether the volume holding `root` can vouch for file contents. Others,
/// such as FAT and exFAT without stable file IDs or change times, get no
/// stamp, so stamped checks hash in full there.
pub(crate) fn stampable(root: &Path) -> io::Result<bool> {
    let root = open(root)?;
    let mut name = [0u16; 261];
    // SAFETY: only the file system name is requested, into `name`'s length.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            root.as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    Ok(ok != 0 && stamps(&name[..len]))
}

/// NTFS, or ReFS (including Dev Drives).
pub(crate) fn stamps(file_system: &[u16]) -> bool {
    ["NTFS", "ReFS"]
        .iter()
        .any(|name| name.encode_utf16().eq(file_system.iter().copied()))
}

/// One file's stamp: its volume and file ID, so a replacement never matches,
/// its size and link count, and its attributes, last-write time and change
/// time. A write advances both times; setting the last-write time back
/// advances the change time again.
pub(crate) fn update(file: &File, hash: &mut Sha256) -> io::Result<()> {
    let id: FILE_ID_INFO = query(file)?;
    let basic: FILE_BASIC_INFO = query(file)?;
    let standard: FILE_STANDARD_INFO = query(file)?;
    hash.update(id.VolumeSerialNumber.to_le_bytes());
    hash.update(id.FileId.Identifier);
    hash.update(standard.EndOfFile.to_le_bytes());
    hash.update(standard.NumberOfLinks.to_le_bytes());
    hash.update(basic.FileAttributes.to_le_bytes());
    hash.update(basic.LastWriteTime.to_le_bytes());
    hash.update(basic.ChangeTime.to_le_bytes());
    Ok(())
}

/// A plain-integer structure and the information class that fills it.
trait Info {
    const CLASS: FILE_INFO_BY_HANDLE_CLASS;
}

impl Info for FILE_ID_INFO {
    const CLASS: FILE_INFO_BY_HANDLE_CLASS = FileIdInfo;
}

impl Info for FILE_BASIC_INFO {
    const CLASS: FILE_INFO_BY_HANDLE_CLASS = FileBasicInfo;
}

impl Info for FILE_STANDARD_INFO {
    const CLASS: FILE_INFO_BY_HANDLE_CLASS = FileStandardInfo;
}

fn query<T: Info>(file: &File) -> io::Result<T> {
    let mut info = MaybeUninit::<T>::zeroed();
    // SAFETY: `T::CLASS` fills exactly one `T`, and the buffer is that size.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            T::CLASS,
            info.as_mut_ptr().cast(),
            size_of::<T>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: plain integers, zeroed and then filled by the call.
    Ok(unsafe { info.assume_init() })
}
