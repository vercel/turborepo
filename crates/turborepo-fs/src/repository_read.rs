//! Read task inputs without following links outside the repository or into Git
//! metadata.

use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, is_git_metadata_component};

/// A read-only repository boundary. Returned files retain the validated handle.
///
/// `resolve` checks link targets, but its result is not an authorization to
/// open a pathname later. Use `open_file` or `read_dir` to validate the opened
/// object.
pub struct RepositoryReader {
    logical_root: PathBuf,
    root: PathBuf,
    directory: File,
}

impl RepositoryReader {
    /// Canonicalize the trusted root once and retain its directory handle.
    pub fn open(root: &AbsoluteSystemPath) -> io::Result<Self> {
        let logical_root = root.as_std_path().to_owned();
        let root = fs::canonicalize(&logical_root)?;
        let directory = open_root(&root)?;
        if !directory.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "repository root",
            ));
        }
        let reader = Self {
            logical_root,
            root,
            directory,
        };
        reader.validate_handle(&reader.directory, false)?;
        Ok(reader)
    }

    /// Resolve at most 40 links, rejecting every external or Git metadata
    /// route.
    pub fn resolve(&self, path: &AbsoluteSystemPath) -> io::Result<AbsoluteSystemPathBuf> {
        let mut pending = components(self.relative(path.as_std_path())?, false)?;
        let mut resolved = self.root.clone();
        let mut links = 0;
        while let Some(component) = pending.pop_front() {
            if component == ".." {
                if resolved == self.root {
                    return Err(boundary());
                }
                resolved.pop();
                continue;
            }
            let candidate = resolved.join(&component);
            let metadata = fs::symlink_metadata(&candidate)?;
            if metadata.file_type().is_symlink() {
                links += 1;
                if links > 40 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "symlink limit exceeded",
                    ));
                }
                let target = fs::read_link(&candidate)?;
                let mut target_components = if target.is_absolute() {
                    let parts = components(self.relative(&target)?, false)?;
                    resolved.clone_from(&self.root);
                    parts
                } else {
                    components(&target, false)?
                };
                target_components.append(&mut pending);
                pending = target_components;
            } else {
                // Windows junctions and other unrecognized reparse points fail closed.
                reject_reparse(&metadata)?;
                if !pending.is_empty() && !metadata.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "input ancestor",
                    ));
                }
                resolved = candidate;
            }
        }
        AbsoluteSystemPath::from_std_path(&resolved)
            .map(ToOwned::to_owned)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Open a regular file for reading. Directories and special files are
    /// skipped. Missing paths remain `NotFound`; boundary violations are
    /// `PermissionDenied`.
    pub fn open_file(&self, path: &AbsoluteSystemPath) -> io::Result<Option<File>> {
        let resolved = self.resolve(path)?;
        if !fs::symlink_metadata(resolved.as_std_path())?.is_file() {
            return Ok(None);
        }
        let (file, _parents) = self.open_resolved(resolved.as_std_path(), false, false)?;
        self.validate_handle(&file, false)?;
        if !file.metadata()?.is_file() {
            return Ok(None);
        }
        Ok(Some(file))
    }

    /// Open an explicitly configured ordinary file, including Git metadata.
    ///
    /// Unlike `open_file`, this never resolves links: every component must be
    /// opened without following links, and every handle must remain under this
    /// reader's trusted root. Directories and opened special files are skipped.
    pub fn open_file_without_links(&self, path: &AbsoluteSystemPath) -> io::Result<Option<File>> {
        let direct = self.root.join(self.relative(path.as_std_path())?);
        let (file, _parents) = self.open_resolved(&direct, false, true)?;
        self.validate_handle(&file, true)?;
        if !file.metadata()?.is_file() {
            return Ok(None);
        }
        Ok(Some(file))
    }

    /// Enumerate names only after validating the opened directory handle.
    pub fn read_dir(&self, path: &AbsoluteSystemPath) -> io::Result<Vec<String>> {
        let resolved = self.resolve(path)?;
        let (directory, _parents) = self.open_resolved(resolved.as_std_path(), true, false)?;
        let actual = self.validate_handle(&directory, false)?;
        if !directory.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "input directory",
            ));
        }
        directory_entries(&directory, &actual)
    }

    fn relative<'a>(&self, path: &'a Path) -> io::Result<&'a Path> {
        path.strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.logical_root))
            .map_err(|_| boundary())
    }

    fn validate_handle(&self, file: &File, allow_metadata: bool) -> io::Result<PathBuf> {
        let actual = final_path(file)?;
        let suffix = actual.strip_prefix(&self.root).map_err(|_| boundary())?;
        // Native final paths are absolute and normalized; do not fall back to a
        // pathname check when the platform cannot identify the opened object.
        components(suffix, allow_metadata)?;
        Ok(actual)
    }

    #[cfg(unix)]
    fn open_resolved(
        &self,
        path: &Path,
        directory: bool,
        allow_metadata: bool,
    ) -> io::Result<(File, Vec<File>)> {
        use std::os::fd::{AsRawFd, FromRawFd};

        let mut file = self.directory.try_clone()?;
        self.validate_handle(&file, allow_metadata)?;
        let mut parts = components(
            path.strip_prefix(&self.root).map_err(|_| boundary())?,
            allow_metadata,
        )?;
        if parts.is_empty() {
            // A new open description keeps root enumeration offsets independent.
            parts.push_back(OsString::from("."));
        }
        let count = parts.len();
        for (index, part) in parts.into_iter().enumerate() {
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::CString::new(part.as_bytes())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
            if directory || index + 1 < count {
                flags |= libc::O_DIRECTORY;
            }
            // SAFETY: name is NUL-terminated; file owns the live parent descriptor.
            let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
            if fd == -1 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: openat returned a new, owned descriptor.
            file = unsafe { File::from_raw_fd(fd) };
            self.validate_handle(&file, allow_metadata)?;
        }
        Ok((file, Vec::new()))
    }

    #[cfg(windows)]
    fn open_resolved(
        &self,
        path: &Path,
        directory: bool,
        allow_metadata: bool,
    ) -> io::Result<(File, Vec<File>)> {
        let mut parents = Vec::new();
        let mut current = self.root.clone();
        let parts = components(
            path.strip_prefix(&self.root).map_err(|_| boundary())?,
            allow_metadata,
        )?;
        let count = parts.len();
        self.validate_handle(&self.directory, allow_metadata)?;
        for (index, part) in parts.into_iter().enumerate() {
            current.push(part);
            let file = open_root(&current)?;
            reject_reparse(&file.metadata()?)?;
            self.validate_handle(&file, allow_metadata)?;
            if index + 1 == count {
                if directory && !file.metadata()?.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "input directory",
                    ));
                }
                return Ok((file, parents));
            }
            if !file.metadata()?.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "input ancestor",
                ));
            }
            parents.push(file);
        }
        Ok((self.directory.try_clone()?, parents))
    }

    #[cfg(not(any(unix, windows)))]
    fn open_resolved(&self, _: &Path, _: bool, _: bool) -> io::Result<(File, Vec<File>)> {
        Err(unsupported())
    }
}

fn components(path: &Path, allow_metadata: bool) -> io::Result<VecDeque<OsString>> {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::ParentDir => Some(Ok(OsString::from(".."))),
            Component::Normal(name) => Some(match name.to_str() {
                Some(name) if allow_metadata || !is_git_metadata_component(name) => {
                    Ok(OsString::from(name))
                }
                Some(_) => Err(boundary()),
                None => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "non-UTF-8 input path",
                )),
            }),
            _ => Some(Err(boundary())),
        })
        .collect()
}

#[cfg(any(target_os = "linux", windows))]
fn directory_entries(directory: &File, actual: &Path) -> io::Result<Vec<String>> {
    #[cfg(target_os = "linux")]
    let path = {
        use std::os::fd::AsRawFd;
        let _ = actual;
        PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
    };
    // Retained ancestor handles exclude FILE_SHARE_DELETE until enumeration ends.
    #[cfg(windows)]
    let path = {
        let _ = directory;
        actual.to_owned()
    };
    fs::read_dir(path)?
        .map(|entry| {
            entry?.file_name().into_string().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 directory entry")
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn directory_entries(directory: &File, _: &Path) -> io::Result<Vec<String>> {
    use std::{ffi::CStr, os::fd::IntoRawFd};

    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            // SAFETY: fdopendir transferred ownership to this directory stream.
            unsafe {
                libc::closedir(self.0);
            }
        }
    }

    // Darwin's /dev/fd entries cannot be opened by read_dir. Enumerate the
    // validated handle directly, never its pathname.
    let fd = directory.try_clone()?.into_raw_fd();
    // SAFETY: fd is a fresh owned duplicate of a validated directory descriptor.
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let error = io::Error::last_os_error();
        // SAFETY: fdopendir failed and did not take ownership of fd.
        unsafe {
            libc::close(fd);
        }
        return Err(error);
    }
    let stream = Directory(stream);
    let mut names = Vec::new();
    loop {
        // SAFETY: __error returns this thread's errno; stream remains live.
        let entry = unsafe {
            *libc::__error() = 0;
            libc::readdir(stream.0)
        };
        if entry.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(0) {
                Ok(names)
            } else {
                Err(error)
            };
        }
        // SAFETY: readdir's entry is live until the next call, with a NUL-terminated
        // name.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let name = name
            .to_str()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if name != "." && name != ".." {
            names.push(name.to_owned());
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn directory_entries(_: &File, _: &Path) -> io::Result<Vec<String>> {
    Err(unsupported())
}

fn boundary() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "input leaves repository or enters Git metadata",
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "native handle paths unavailable",
    )
}

#[cfg(unix)]
fn open_root(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(windows)]
fn open_root(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_root(_: &Path) -> io::Result<File> {
    Err(unsupported())
}

fn reject_reparse(metadata: &fs::Metadata) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(boundary());
        }
    }
    let _ = metadata;
    Ok(())
}

#[cfg(target_os = "linux")]
fn final_path(file: &File) -> io::Result<PathBuf> {
    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
    let path = fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    // This suffix can also be an ordinary authored filename. Only an unlinked
    // object with the procfs marker is rejected.
    if path.as_os_str().as_encoded_bytes().ends_with(b" (deleted)") && file.metadata()?.nlink() == 0
    {
        return Err(boundary());
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
fn final_path(file: &File) -> io::Result<PathBuf> {
    use std::{
        ffi::CStr,
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
    };
    let mut buffer = [0 as libc::c_char; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes to this live buffer.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful F_GETPATH returns a NUL-terminated path.
    let path = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

#[cfg(windows)]
fn final_path(file: &File) -> io::Result<PathBuf> {
    use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};

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
        if (len as usize) < buffer.len() {
            return Ok(PathBuf::from(OsString::from_wide(&buffer[..len as usize])));
        }
        buffer.resize(len as usize + 1, 0);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn final_path(_: &File) -> io::Result<PathBuf> {
    Err(unsupported())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Read;

    use super::*;

    fn absolute(path: &Path) -> &AbsoluteSystemPath {
        AbsoluteSystemPath::from_std_path(path).unwrap()
    }

    fn fixture() -> (tempfile::TempDir, RepositoryReader) {
        let temp = tempfile::tempdir().unwrap();
        let reader = RepositoryReader::open(absolute(temp.path())).unwrap();
        fs::create_dir(temp.path().join("dir")).unwrap();
        fs::write(temp.path().join("dir/input"), "input contents").unwrap();
        (temp, reader)
    }

    #[cfg(unix)]
    fn link(target: &Path, path: &Path, _: bool) {
        std::os::unix::fs::symlink(target, path).unwrap();
    }

    #[cfg(windows)]
    fn link(target: &Path, path: &Path, directory: bool) {
        if directory {
            std::os::windows::fs::symlink_dir(target, path).unwrap();
        } else {
            std::os::windows::fs::symlink_file(target, path).unwrap();
        }
    }

    #[test]
    fn direct_metadata_files_are_read_without_links() {
        let (temp, reader) = fixture();
        fs::create_dir(temp.path().join(".git")).unwrap();
        let path = temp.path().join(".git/index.lock");
        fs::write(&path, "ordinary metadata").unwrap();
        let mut file = reader
            .open_file_without_links(absolute(&path))
            .unwrap()
            .unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "ordinary metadata");
        assert_eq!(
            reader.open_file(absolute(&path)).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(
            reader
                .open_file_without_links(absolute(&temp.path().join(".git")))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            reader
                .open_file_without_links(absolute(&temp.path().join(".git/missing")))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn direct_metadata_files_reject_parent_and_leaf_links() {
        let (temp, reader) = fixture();
        fs::create_dir(temp.path().join(".git")).unwrap();
        link(
            Path::new("../dir/input"),
            &temp.path().join(".git/leaf"),
            false,
        );
        link(Path::new("../dir"), &temp.path().join(".git/parent"), true);
        link(Path::new("dir"), &temp.path().join("parent"), true);
        for name in [".git/leaf", ".git/parent/input", "parent/input"] {
            assert!(
                reader
                    .open_file_without_links(absolute(&temp.path().join(name)))
                    .is_err(),
                "{name}"
            );
        }
        assert!(
            reader
                .open_file_without_links(absolute(&temp.path().join("dir/input")))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn direct_metadata_files_remain_inside_reader_root() {
        let (temp, reader) = fixture();
        let external = tempfile::tempdir().unwrap();
        let path = external.path().join("outside");
        fs::write(&path, "external").unwrap();
        assert_eq!(
            reader
                .open_file_without_links(absolute(&path))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            reader
                .open_file_without_links(absolute(&temp.path().join("../outside")))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::create_dir(temp.path().join(".git")).unwrap();
        link(&path, &temp.path().join(".git/outside"), false);
        assert!(
            reader
                .open_file_without_links(absolute(&temp.path().join(".git/outside")))
                .is_err()
        );
    }

    #[test]
    fn authored_deleted_suffix_is_a_valid_filename() {
        let (temp, reader) = fixture();
        let path = temp.path().join("input (deleted)");
        fs::write(&path, "authored contents").unwrap();
        for mut file in [
            reader.open_file(absolute(&path)).unwrap().unwrap(),
            reader
                .open_file_without_links(absolute(&path))
                .unwrap()
                .unwrap(),
        ] {
            let mut text = String::new();
            file.read_to_string(&mut text).unwrap();
            assert_eq!(text, "authored contents");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unlinked_handle_is_rejected() {
        let (temp, reader) = fixture();
        let path = temp.path().join("dir/input");
        let file = File::open(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(
            reader.validate_handle(&file, true).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn regular_inputs_and_missing() {
        let (temp, reader) = fixture();
        let mut file = reader
            .open_file(absolute(&temp.path().join("dir/input")))
            .unwrap()
            .unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "input contents");
        assert_eq!(
            reader.read_dir(absolute(&temp.path().join("dir"))).unwrap(),
            ["input"]
        );
        assert!(
            reader
                .open_file(absolute(&temp.path().join("dir")))
                .unwrap()
                .is_none()
        );
        for result in [
            reader
                .resolve(absolute(&temp.path().join("missing")))
                .map(|_| ()),
            reader
                .open_file(absolute(&temp.path().join("missing")))
                .map(|_| ()),
            reader
                .read_dir(absolute(&temp.path().join("missing")))
                .map(|_| ()),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
        }
    }

    #[test]
    fn internal_relative_and_absolute_links() {
        let (temp, reader) = fixture();
        for (name, target, directory) in [
            ("relative-file", PathBuf::from("dir/input"), false),
            ("absolute-file", temp.path().join("dir/input"), false),
            ("relative-dir", PathBuf::from("dir"), true),
            ("absolute-dir", temp.path().join("dir"), true),
        ] {
            let path = temp.path().join(name);
            link(&target, &path, directory);
            assert!(reader.resolve(absolute(&path)).is_ok());
            if directory {
                assert_eq!(reader.read_dir(absolute(&path)).unwrap(), ["input"]);
                assert!(
                    reader
                        .open_file(absolute(&path.join("input")))
                        .unwrap()
                        .is_some()
                );
            } else {
                let mut text = String::new();
                reader
                    .open_file(absolute(&path))
                    .unwrap()
                    .unwrap()
                    .read_to_string(&mut text)
                    .unwrap();
                assert_eq!(text, "input contents");
            }
        }
        link(
            Path::new("relative-file"),
            &temp.path().join("chain"),
            false,
        );
        assert!(
            reader
                .open_file(absolute(&temp.path().join("chain")))
                .unwrap()
                .is_some()
        );
        link(
            Path::new("../dir/input"),
            &temp.path().join("dir/up"),
            false,
        );
        assert!(
            reader
                .open_file(absolute(&temp.path().join("dir/up")))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn rejects_external_and_git_routes() {
        let (temp, reader) = fixture();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("secret"), "external").unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git/secret"), "metadata").unwrap();
        link(
            &external.path().join("secret"),
            &temp.path().join("outside"),
            false,
        );
        link(external.path(), &temp.path().join("outside-dir"), true);
        link(
            &temp.path().join("dir/input"),
            &external.path().join("back"),
            false,
        );
        link(
            &external.path().join("back"),
            &temp.path().join("roundtrip"),
            false,
        );
        link(Path::new(".git"), &temp.path().join("git-dir"), true);
        link(
            Path::new("git-dir/secret"),
            &temp.path().join("git-chain"),
            false,
        );
        link(
            Path::new(".git/../dir/input"),
            &temp.path().join("git-detour"),
            false,
        );
        link(
            Path::new("dir/input"),
            &temp.path().join(".git/back"),
            false,
        );
        for name in [
            "outside",
            "outside-dir/secret",
            "roundtrip",
            ".git/secret",
            ".git/back",
            ".git/../dir/input",
            "git-dir/secret",
            "git-chain",
            "git-detour",
            "../escape",
        ] {
            let path = temp.path().join(name);
            assert_eq!(
                reader.resolve(absolute(&path)).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied,
                "{name}"
            );
            assert_eq!(
                reader.open_file(absolute(&path)).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied,
                "{name}"
            );
            assert_eq!(
                reader.read_dir(absolute(&path)).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied,
                "{name}"
            );
        }
        assert_eq!(
            reader
                .read_dir(absolute(&temp.path().join("git-dir")))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn cycles_and_long_chains_are_bounded() {
        let (temp, reader) = fixture();
        link(Path::new("b"), &temp.path().join("a"), false);
        link(Path::new("a"), &temp.path().join("b"), false);
        assert_eq!(
            reader
                .resolve(absolute(&temp.path().join("a")))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        for i in 0..41 {
            let target = if i == 40 {
                PathBuf::from("dir/input")
            } else {
                PathBuf::from(format!("link{}", i + 1))
            };
            link(&target, &temp.path().join(format!("link{i}")), false);
        }
        assert_eq!(
            reader
                .resolve(absolute(&temp.path().join("link0")))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(
            reader
                .open_file(absolute(&temp.path().join("link1")))
                .unwrap()
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn special_files_are_skipped_without_blocking() {
        use std::os::unix::{ffi::OsStrExt, net::UnixListener};
        let (temp, reader) = fixture();
        let fifo = temp.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is NUL-terminated and mkfifo does not retain it.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let socket = temp.path().join("socket");
        let _listener = UnixListener::bind(&socket).unwrap();
        for path in [fifo, socket] {
            assert!(reader.open_file(absolute(&path)).unwrap().is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn returned_file_survives_path_replacement() {
        let (temp, reader) = fixture();
        let path = temp.path().join("dir/input");
        let mut file = reader.open_file(absolute(&path)).unwrap().unwrap();
        fs::rename(&path, temp.path().join("saved")).unwrap();
        fs::write(&path, "replacement").unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "input contents");
    }

    #[cfg(unix)]
    #[test]
    fn directory_enumeration_survives_path_replacement() {
        let (temp, reader) = fixture();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("secret"), "external").unwrap();
        let resolved = reader.resolve(absolute(&temp.path().join("dir"))).unwrap();
        let (directory, _parents) = reader
            .open_resolved(resolved.as_std_path(), true, false)
            .unwrap();
        let actual = reader.validate_handle(&directory, false).unwrap();
        fs::rename(temp.path().join("dir"), temp.path().join("saved")).unwrap();
        link(external.path(), &temp.path().join("dir"), true);
        assert_eq!(directory_entries(&directory, &actual).unwrap(), ["input"]);
    }

    #[test]
    fn root_enumeration_is_repeatable() {
        let (temp, reader) = fixture();
        for _ in 0..3 {
            assert_eq!(reader.read_dir(absolute(temp.path())).unwrap(), ["dir"]);
        }
    }

    #[test]
    fn metadata_aliases_are_denied_before_lookup() {
        let (temp, reader) = fixture();
        for name in [".GIT", ".g\u{200d}it", ".git\u{feff}"] {
            let path = temp.path().join(name).join("secret");
            assert_eq!(
                reader.resolve(absolute(&path)).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
    }

    #[test]
    fn trusted_root_can_be_a_directory_link() {
        let (temp, _reader) = fixture();
        let aliases = tempfile::tempdir().unwrap();
        let alias = aliases.path().join("repository");
        link(temp.path(), &alias, true);
        let reader = RepositoryReader::open(absolute(&alias)).unwrap();
        assert!(
            reader
                .open_file(absolute(&alias.join("dir/input")))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            reader.read_dir(absolute(&alias.join("dir"))).unwrap(),
            ["input"]
        );
    }

    #[test]
    fn replacement_links_are_not_followed_after_resolution() {
        let (temp, reader) = fixture();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("input"), "external").unwrap();
        let file = reader
            .resolve(absolute(&temp.path().join("dir/input")))
            .unwrap();
        let dir = reader.resolve(absolute(&temp.path().join("dir"))).unwrap();
        fs::rename(temp.path().join("dir"), temp.path().join("saved")).unwrap();
        link(external.path(), &temp.path().join("dir"), true);
        assert!(
            reader
                .open_resolved(file.as_std_path(), false, false)
                .is_err()
        );
        assert!(
            reader
                .open_resolved(dir.as_std_path(), true, false)
                .is_err()
        );

        let final_file = reader
            .resolve(absolute(&temp.path().join("saved/input")))
            .unwrap();
        fs::rename(
            temp.path().join("saved/input"),
            temp.path().join("original"),
        )
        .unwrap();
        link(
            &external.path().join("input"),
            &temp.path().join("saved/input"),
            false,
        );
        assert!(
            reader
                .open_resolved(final_file.as_std_path(), false, false)
                .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn returned_file_prevents_path_replacement() {
        let (temp, reader) = fixture();
        let path = temp.path().join("dir/input");
        let mut file = reader.open_file(absolute(&path)).unwrap().unwrap();
        assert!(fs::rename(&path, temp.path().join("saved")).is_err());
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "input contents");
    }

    #[test]
    fn final_handle_rejects_external_and_git_objects() {
        let (temp, reader) = fixture();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("secret"), "external").unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::write(temp.path().join(".git/secret"), "metadata").unwrap();
        for path in [
            external.path().join("secret"),
            temp.path().join(".git/secret"),
        ] {
            let file = File::open(path).unwrap();
            assert_eq!(
                reader.validate_handle(&file, false).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
    }
}
