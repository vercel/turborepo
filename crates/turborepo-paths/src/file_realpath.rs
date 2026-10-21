use std::{fs::File, io, path::PathBuf};

/// Resolve the opened object, rather than reopening a path that may have
/// changed.
#[cfg(target_os = "linux")]
pub fn file_realpath(file: &File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(target_os = "macos")]
pub fn file_realpath(file: &File) -> io::Result<PathBuf> {
    use std::{
        ffi::CStr,
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };
    let mut path = [0 as libc::c_char; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes a NUL-terminated path into a PATH_MAX buffer.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, path.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(
        unsafe { CStr::from_ptr(path.as_ptr()) }.to_bytes(),
    )))
}

#[cfg(windows)]
pub fn file_realpath(file: &File) -> io::Result<PathBuf> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };
    let mut buffer = vec![0u16; 260];
    loop {
        // SAFETY: file owns a live handle and buffer has the advertised capacity.
        let len = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        let len = len as usize;
        if len < buffer.len() {
            let path = String::from_utf16(&buffer[..len])
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            return Ok(dunce::simplified(std::path::Path::new(&path)).to_owned());
        }
        buffer.resize(len + 1, 0);
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_opened_file_after_path_replacement() {
        let tmp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        let original = root.join("original");
        let moved = root.join("moved");
        std::fs::write(&original, "opened").unwrap();
        let file = File::open(&original).unwrap();
        std::fs::rename(&original, &moved).unwrap();
        std::fs::write(&original, "replacement").unwrap();
        assert_eq!(file_realpath(&file).unwrap(), moved);
        assert_ne!(file_realpath(&file).unwrap(), original);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn file_realpath(_: &File) -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "opened-file paths are unavailable",
    ))
}
