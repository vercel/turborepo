//! The repository's `.turbo`, which any account that can write the checkout
//! root can rename or replace with a link. Setup checks it once through a
//! handle and then works only through that handle, so a swap after the check
//! redirects neither the editor link nor the removal of pre-store state into
//! a directory outside the repository.

#[cfg(windows)]
use std::{fs::File, path::PathBuf};

/// A checked `.turbo` that cannot be swapped while this value lives: a
/// descriptor everything is resolved relative to on Unix, and a handle that
/// denies rename and delete on Windows.
pub(crate) struct RepoTurbo {
    #[cfg(unix)]
    dir: rustix::fd::OwnedFd,
    #[cfg(windows)]
    _pin: File,
    #[cfg(windows)]
    path: PathBuf,
}

#[cfg(unix)]
mod unix {
    use std::{
        ffi::{CStr, CString, OsStr},
        io,
        os::unix::ffi::OsStrExt,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use rustix::{
        fd::{AsFd, BorrowedFd},
        fs::{self, AtFlags, Dir, FileType, Mode, OFlags},
        io::Errno,
    };

    use super::RepoTurbo;
    use crate::Error;

    const DIRECTORY: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    fn io_error(e: Errno) -> Error {
        Error::Io(io::Error::from(e))
    }

    impl RepoTurbo {
        /// Opens `<repo>/.turbo`, creating it owner-only when missing. The
        /// repository root and `.turbo` keep the modes the user's umask gave
        /// them, so only a world-writable `.turbo` is rejected.
        pub(crate) fn open(repo: &Path) -> Result<Self, Error> {
            let repo = fs::open(
                repo,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(io_error)?;
            match fs::mkdirat(&repo, ".turbo", Mode::from_raw_mode(0o700)) {
                Ok(()) => fs::fsync(&repo).map_err(io_error)?,
                Err(Errno::EXIST) => {}
                Err(e) => return Err(io_error(e)),
            }
            let dir = match fs::openat(&repo, ".turbo", DIRECTORY, Mode::empty()) {
                Ok(dir) => dir,
                Err(Errno::LOOP | Errno::NOTDIR) => return Err(Error::UnsafePath),
                Err(e) => return Err(io_error(e)),
            };
            if fs::fstat(&dir).map_err(io_error)?.st_mode & 0o002 != 0 {
                return Err(Error::UnsafePath);
            }
            Ok(Self { dir })
        }

        /// Points `.turbo/bin` at `stable`, never replacing a directory or
        /// file someone else put there.
        pub(crate) fn link_editor(&self, stable: &Path) -> Result<(), Error> {
            match fs::statat(&self.dir, "bin", AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) if FileType::from_raw_mode(st.st_mode) != FileType::Symlink => {
                    return Ok(());
                }
                Ok(_) => {
                    let current = fs::readlinkat(&self.dir, "bin", Vec::new()).map_err(io_error)?;
                    if current.as_bytes() == stable.as_os_str().as_bytes() {
                        return Ok(());
                    }
                }
                Err(Errno::NOENT) => {}
                Err(e) => return Err(io_error(e)),
            }
            let staged = self.stage_link(stable)?;
            if let Err(e) = fs::renameat(&self.dir, &staged, &self.dir, "bin") {
                let _ = fs::unlinkat(&self.dir, &staged, AtFlags::empty());
                return Err(io_error(e));
            }
            fs::fsync(&self.dir).map_err(io_error)
        }

        /// A new symlink to `target` under a fresh `.link-*` name.
        fn stage_link(&self, target: &Path) -> Result<CString, Error> {
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            for attempt in 0u32..64 {
                let name = format!(".link-{}-{seed:x}-{attempt}", std::process::id());
                let name = CString::new(name).map_err(|_| Error::UnsafePath)?;
                match fs::symlinkat(target, &self.dir, &name) {
                    Ok(()) => return Ok(name),
                    Err(Errno::EXIST) => continue,
                    Err(e) => return Err(io_error(e)),
                }
            }
            Err(Error::Io(io::Error::from(io::ErrorKind::AlreadyExists)))
        }

        /// Removes the pre-store `.turbo/tools`. Best effort, like any other
        /// removal a running process may still hold open.
        pub(crate) fn remove_legacy_tools(&self) {
            if fs::statat(&self.dir, "tools", AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|st| FileType::from_raw_mode(st.st_mode) == FileType::Directory)
            {
                let _ = remove_at(self.dir.as_fd(), OsStr::new("tools"));
            }
        }
    }

    /// Removes `name` under `parent` without following any link: a directory
    /// is opened relative to `parent` with `O_NOFOLLOW` and emptied through
    /// that descriptor, and anything else, links included, is unlinked.
    fn remove_at(parent: BorrowedFd<'_>, name: &OsStr) -> rustix::io::Result<()> {
        let dir = match fs::openat(parent, name, DIRECTORY, Mode::empty()) {
            Ok(dir) => dir,
            Err(Errno::LOOP | Errno::NOTDIR) => {
                return fs::unlinkat(parent, name, AtFlags::empty());
            }
            Err(e) => return Err(e),
        };
        let children = Dir::read_from(&dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let child = entry.file_name();
                (child != c"." && child != c"..").then(|| child.to_owned())
            })
            .collect::<Vec<CString>>();
        for child in children {
            let _ = remove_at(dir.as_fd(), OsStr::from_bytes(CStr::to_bytes(&child)));
        }
        fs::unlinkat(parent, name, AtFlags::REMOVEDIR)
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        fs::{self, OpenOptions},
        os::windows::fs::{MetadataExt, OpenOptionsExt},
        path::Path,
    };

    use super::RepoTurbo;
    use crate::{Error, create_directory, shared_directory};

    // FILE_SHARE_READ | FILE_SHARE_WRITE: no FILE_SHARE_DELETE.
    const SHARE_NO_DELETE: u32 = 0x1 | 0x2;
    // FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS.
    const OPEN_DIRECTORY_ITSELF: u32 = 0x0020_0000 | 0x0200_0000;
    const REPARSE_POINT: u32 = 0x400;

    impl RepoTurbo {
        /// Opens `<repo>\.turbo`, creating it when missing, and holds it
        /// without delete sharing: nobody can rename or remove it, or put a
        /// junction in its place, until this value drops.
        pub(crate) fn open(repo: &Path) -> Result<Self, Error> {
            let path = repo.join(".turbo");
            create_directory(&path, shared_directory)?;
            let pin = OpenOptions::new()
                .read(true)
                .share_mode(SHARE_NO_DELETE)
                .custom_flags(OPEN_DIRECTORY_ITSELF)
                .open(&path)?;
            let m = pin.metadata()?;
            if !m.is_dir() || m.file_attributes() & REPARSE_POINT != 0 {
                return Err(Error::UnsafePath);
            }
            Ok(Self { _pin: pin, path })
        }

        /// Points `.turbo\bin` at `stable`, never replacing a directory or
        /// file someone else put there.
        pub(crate) fn link_editor(&self, stable: &Path) -> Result<(), Error> {
            let editor = self.path.join("bin");
            match fs::symlink_metadata(&editor) {
                Ok(m) if !m.file_type().is_symlink() => Ok(()),
                _ => crate::store::replace_link(&editor, stable, stable),
            }
        }

        /// Removes the pre-store `.turbo\tools`. Best effort, like any other
        /// removal a running process may still hold open. The standard
        /// removal never follows a reparse point beneath it.
        pub(crate) fn remove_legacy_tools(&self) {
            let old = self.path.join("tools");
            if fs::symlink_metadata(&old).is_ok_and(|m| m.is_dir()) {
                let _ = fs::remove_dir_all(&old);
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
impl RepoTurbo {
    pub(crate) fn open(_repo: &std::path::Path) -> Result<Self, crate::Error> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub(crate) fn link_editor(&self, _stable: &std::path::Path) -> Result<(), crate::Error> {
        Err(crate::Error::UnsupportedPlatform)
    }

    pub(crate) fn remove_legacy_tools(&self) {}
}
