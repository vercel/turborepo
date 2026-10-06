//! Writer-only bookkeeping; readers must not acquire or initialize storage.
//! Requires Git-proven ignore coverage. Missing coverage is reported before
//! creating anything; the setup command owns any explicit ignore-file edits.

#[cfg(windows)]
use std::{fs, path::PathBuf};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
    process::Command,
};

const MAX_BYTES: usize = 1024 * 1024;
const WRITER: &str = "writer";
const STAGED: &str = "staged";
const TARGET: &str = "turbo.lock";

struct Directory {
    file: File,
    #[cfg(windows)]
    path: PathBuf,
}

fn validate(file: &File, directory: bool) -> io::Result<()> {
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("setup storage is a reparse point"));
        }
    }
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(io::Error::other("invalid setup storage file type"));
    }
    Ok(())
}

impl Directory {
    fn root(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        let file = File::from(rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )?);
        #[cfg(windows)]
        let file = Self::windows_open(path, true, false, false)?;
        validate(&file, true)?;
        Ok(Self {
            file,
            #[cfg(windows)]
            path: path.into(),
        })
    }

    fn child(&self, name: &str) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags, mkdirat, openat};
            match mkdirat(&self.file, name, Mode::from_raw_mode(0o700)) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => {}
                Err(error) => return Err(error.into()),
            }
            let file = File::from(openat(
                &self.file,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?);
            validate(&file, true)?;
            Ok(Self { file })
        }
        #[cfg(windows)]
        {
            let path = self.path.join(name);
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            Self::root(&path)
        }
    }

    #[cfg(windows)]
    fn windows_open(
        path: &Path,
        directory: bool,
        create: bool,
        exclusive: bool,
    ) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        let mut options = fs::OpenOptions::new();
        options
            .read(true)
            .write(create)
            .create(create && !exclusive)
            .create_new(exclusive)
            .custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 });
        if directory || create && !exclusive {
            // Pin every directory and the persistent writer inode against deletion.
            options.share_mode(0x1 | 0x2);
        }
        let file = options.open(path)?;
        validate(&file, directory)?;
        Ok(file)
    }

    fn open(&self, name: &str, create: bool, exclusive: bool) -> io::Result<File> {
        #[cfg(unix)]
        let file = {
            use rustix::fs::{Mode, OFlags, openat};
            let mut flags = OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
            flags |= if create {
                OFlags::RDWR | OFlags::CREATE
            } else {
                OFlags::RDONLY
            };
            if exclusive {
                flags |= OFlags::EXCL;
            }
            File::from(openat(&self.file, name, flags, Mode::from_raw_mode(0o600))?)
        };
        #[cfg(windows)]
        let file = Self::windows_open(&self.path.join(name), false, create, exclusive)?;
        validate(&file, false)?;
        Ok(file)
    }

    fn optional(&self, name: &str) -> io::Result<Option<File>> {
        match self.open(name, false, false) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn remove_stage(&self) -> io::Result<()> {
        #[cfg(unix)]
        rustix::fs::unlinkat(&self.file, STAGED, rustix::fs::AtFlags::empty())?;
        #[cfg(windows)]
        fs::remove_file(self.path.join(STAGED))?;
        Ok(())
    }

    fn promote(&self, root: &Self) -> io::Result<()> {
        #[cfg(unix)]
        rustix::fs::renameat(&self.file, STAGED, &root.file, TARGET)?;
        #[cfg(windows)]
        fs::rename(self.path.join(STAGED), root.path.join(TARGET))?;
        Ok(())
    }
}

/// Exclusive transaction guard. The persistent writer file is never removed.
/// All operations use pinned directory handles, not unchecked directory paths.
/// No schema/resolver work happens here; callers compare snapshots under this
/// guard before promotion. Root ancestors are caller-selected and stable.
pub struct WriterStorage {
    root: Directory,
    _cache: Directory,
    storage: Directory,
    _guard: File,
}

impl WriterStorage {
    pub fn acquire(root: &Path) -> io::Result<Self> {
        let root_path = root.canonicalize()?;
        let root = Directory::root(&root_path)?;
        // Unix permits renaming an open cache directory. Lock the stable root
        // inode first so a recreated cache cannot split concurrent writers.
        // Windows directory handles already deny deletion of the pinned chain.
        #[cfg(unix)]
        root.file.lock()?;
        let paths = [".turbo/setup-lock/writer", ".turbo/setup-lock/staged"];
        let output = Command::new("git")
            .current_dir(&root_path)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(["check-ignore", "--"])
            .args(paths)
            .output()?;
        if !output.status.success()
            || output.stdout != format!("{}\n{}\n", paths[0], paths[1]).as_bytes()
        {
            return Err(io::Error::other(
                "setup storage must be untracked and ignored by Git; explicitly add /.turbo/ to \
                 .gitignore",
            ));
        }
        let cache = root.child(".turbo")?;
        let storage = cache.child("setup-lock")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if root.file.metadata()?.dev() != storage.file.metadata()?.dev() {
                return Err(io::Error::other(
                    "setup staging must share the lock filesystem",
                ));
            }
        }
        let guard = storage.open(WRITER, true, false)?;
        guard.lock()?;
        // A prior process may have died after creating/flushing its stage.
        // Inspect without following links, then remove only this owned entry.
        if let Some(file) = storage.optional(STAGED)? {
            drop(file);
            storage.remove_stage()?;
        }
        Ok(Self {
            root,
            _cache: cache,
            storage,
            _guard: guard,
        })
    }

    pub fn read_lock(&self) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.root.optional(TARGET)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::other("setup lock exceeds byte limit"));
        }
        Ok(Some(bytes))
    }

    /// Caller supplies a complete, validated bounded selection. Never publish
    /// partial bytes. There is no fallible operation after atomic promotion.
    pub fn replace(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.replace_with(bytes, || Ok(()))
    }

    fn replace_with(
        &mut self,
        bytes: &[u8],
        before_promote: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::other("setup lock exceeds byte limit"));
        }
        self.read_lock()?;
        let mut file = self.storage.open(STAGED, true, true)?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            before_promote()?;
            self.read_lock()?;
            drop(file);
            self.storage.promote(&self.root)
        })();
        if result.is_err() {
            // Cleanup failure leaves only ignored owned state for next acquire.
            let _ = self.storage.remove_stage();
        }
        result
    }
}

#[cfg(test)]
mod tests;
