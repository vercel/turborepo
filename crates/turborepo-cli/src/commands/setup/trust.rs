//! Setup's private producer of user-owned activation evidence. No PATH changes
//! or execution capability. Run consumers must additionally gate managed mode,
//! validate declarations/lock and revalidate immediately before use.
//! Same-user hostile mutation during a transaction is outside the Store
//! contract.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use turborepo_tool_install::{Store, Tool};

const LIMIT: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Installation {
    root: PathBuf,
    device: u64,
    inode: u64,
    manifest_sha256: String,
    bin: PathBuf,
    tools: Vec<Tool>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    lock_sha256: String,
    installation: Installation,
}

pub(super) struct TrustStore {
    directory: PathBuf,
    root: PathBuf,
}

fn unsafe_state() -> io::Error {
    io::Error::other("unsafe setup user state; use a private directory outside the repository")
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// Bounded non-symlink files only. The repo manifest is NOT trusted by this
// read; only equality with a user-state record, plus Store's tree/shim checks,
// permits reuse. User state is additionally private, single-link and owned by
// this user.
fn read(path: &Path, private: bool) -> io::Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > LIMIT {
        return Err(unsafe_state());
    }
    #[cfg(unix)]
    {
        use std::{
            io::Read,
            os::unix::fs::{MetadataExt, OpenOptionsExt},
        };
        if private
            && (metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1)
        {
            return Err(unsafe_state());
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let opened = file.metadata()?;
        if (opened.dev(), opened.ino()) != (metadata.dev(), metadata.ino()) {
            return Err(unsafe_state());
        }
        #[cfg(target_os = "macos")]
        if private {
            super::acl::check(&file, true)?;
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(unsafe_state());
        }
        Ok(Some(bytes))
    }
    #[cfg(not(unix))]
    {
        let _ = private;
        Err(unsafe_state())
    }
}

fn private_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(unsafe_state());
        }
        #[cfg(target_os = "macos")]
        super::acl::check(&File::open(path)?, true)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(unsafe_state())
    }
}

impl TrustStore {
    // The caller chooses a user location, never a repository-authored path.
    // No writes until publication. Missing user state simply prohibits reuse.
    pub(super) fn new(base: &Path, root: &Path) -> io::Result<Self> {
        if !cfg!(unix)
            || !base.is_absolute()
            || base.components().any(|c| matches!(c, Component::ParentDir))
        {
            return Err(unsafe_state());
        }
        let root = root.canonicalize()?;
        // Resolve an existing ancestor without accepting a symlink as a new
        // state component. macOS's /var alias is allowed in the supplied base.
        let mut ancestor = base;
        let mut missing = Vec::new();
        while !ancestor.exists() {
            if fs::symlink_metadata(ancestor).is_ok() {
                return Err(unsafe_state());
            }
            missing.push(ancestor.file_name().ok_or_else(unsafe_state)?.to_owned());
            ancestor = ancestor.parent().ok_or_else(unsafe_state)?;
        }
        let mut directory = ancestor.canonicalize()?;
        for part in missing.into_iter().rev() {
            directory.push(part);
        }
        directory.push("turborepo");
        directory.push("setup-roots");
        if directory.starts_with(&root) || root.starts_with(&directory) {
            return Err(unsafe_state());
        }
        Ok(Self { directory, root })
    }

    fn path(&self) -> PathBuf {
        // Encoded native bytes avoid lossy collisions for non-UTF8 Unix roots.
        self.directory.join(format!(
            "{}.json",
            hash(self.root.as_os_str().as_encoded_bytes())
        ))
    }

    fn load(&self) -> io::Result<Option<Record>> {
        // Reject links in any existing state component before opening a record.
        // Only the record directory must be private: on macOS its parent is
        // turbo's shared config directory, which is commonly created as 0755.
        for path in self.directory.ancestors() {
            match fs::symlink_metadata(path) {
                Ok(metadata) => {
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err(unsafe_state());
                    }
                    #[cfg(target_os = "macos")]
                    super::acl::check(&File::open(path)?, path == self.directory)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        // Root-owned sticky temp ancestors are safe; writable
                        // user-state parents without that boundary are not.
                        let sticky_root = metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
                        if (metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() })
                            || (metadata.mode() & 0o022 != 0 && !sticky_root)
                        {
                            return Err(unsafe_state());
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        match fs::symlink_metadata(&self.directory) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(unsafe_state());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0
                    {
                        return Err(unsafe_state());
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let Some(bytes) = read(&self.path(), true)? else {
            return Ok(None);
        };
        // Corrupt/old records are not authority; a fresh setup can replace them.
        Ok(serde_json::from_slice::<Record>(&bytes)
            .ok()
            .filter(|r| r.schema == 1))
    }

    pub(super) fn preflight(&self) -> io::Result<()> {
        self.load().map(|_| ())
    }

    fn installation(&self, store: &Store) -> io::Result<Option<Installation>> {
        if store.repository_root().map_err(io::Error::other)? != self.root {
            return Err(unsafe_state());
        }
        let path = self.root.join(".turbo/tools/manifest.json");
        let Some(before) = read(&path, false)? else {
            return Ok(None);
        };
        let Some(current) = store.current().map_err(io::Error::other)? else {
            return Ok(None);
        };
        if read(&path, false)?.as_deref() != Some(before.as_slice()) {
            return Err(unsafe_state());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = fs::metadata(&self.root)?;
            Ok(Some(Installation {
                root: self.root.clone(),
                device: metadata.dev(),
                inode: metadata.ino(),
                manifest_sha256: hash(&before),
                bin: current.bin,
                tools: current.tools,
            }))
        }
        #[cfg(not(unix))]
        {
            let _ = current;
            Err(unsafe_state())
        }
    }

    /// Setup-only reuse of previously verified bytes. Deliberately ignores lock
    /// drift: setup can reuse unchanged tools while repairing the lock/cohort.
    /// This MUST NOT be used as run authorization, which also needs lock
    /// equality.
    pub(super) fn allows_reuse(&self, store: &Store) -> io::Result<bool> {
        let Some(record) = self.load()? else {
            return Ok(false);
        };
        // Damaged inventory is repairable, never reusable.
        Ok(self.installation(store).ok().flatten().as_ref() == Some(&record.installation))
    }

    /// Only call under source/writer/Store guards after the cohort was freshly
    /// staged from verified artifacts or reused from matching external
    /// evidence.
    pub(super) fn publish(&self, store: &Store) -> io::Result<()> {
        self.load()?;
        let installation = self.installation(store)?.ok_or_else(unsafe_state)?;
        let lock = read(&self.root.join("turbo.lock"), false)?.ok_or_else(unsafe_state)?;
        let bytes = serde_json::to_vec(&Record {
            schema: 1,
            lock_sha256: hash(&lock),
            installation,
        })
        .map_err(io::Error::other)?;
        if bytes.len() as u64 > LIMIT {
            return Err(unsafe_state());
        }
        if read(&self.path(), true)?.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        // Recheck location on each write; never follow a state symlink introduced
        // since new(). Create missing components with private permissions.
        let mut missing = Vec::new();
        let mut parent = self.directory.as_path();
        while !parent.exists() {
            missing.push(parent.to_owned());
            parent = parent.parent().ok_or_else(unsafe_state)?;
        }
        if parent.canonicalize()? != parent {
            return Err(unsafe_state());
        }
        for path in missing.into_iter().rev() {
            private_directory(&path)?;
        }
        private_directory(&self.directory)?;
        let mut staged = tempfile::NamedTempFile::new_in(&self.directory)?;
        #[cfg(target_os = "macos")]
        super::acl::check(staged.as_file(), true)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged.persist(self.path()).map_err(|e| e.error)?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}
