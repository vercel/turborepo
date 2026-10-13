//! Setup's private producer of user-owned activation evidence. No PATH changes
//! or execution capability. Run consumers must additionally gate managed mode,
//! validate declarations/lock and revalidate immediately before use.
//! Same-user hostile mutation during a transaction is outside the Store
//! contract. Unix privacy uses owner, mode and macOS ACL checks; Windows uses
//! owner-only DACLs read from each opened handle (see `dacl`).

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use turborepo_tool_install::{Store, Tool};

const LIMIT: u64 = 1024 * 1024;
/// Bounds one pruning pass over the user's records.
const MAX_RECORDS: usize = 4096;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Installation {
    root: PathBuf,
    /// Unix device and inode, or Windows volume serial number and file index.
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

pub(crate) struct TrustStore {
    directory: PathBuf,
    root: PathBuf,
}

/// Setup-recorded tools for the interactive shell hook only.
pub(crate) struct Recorded {
    pub(crate) bin: PathBuf,
    /// turbo.lock changed since setup recorded this installation.
    pub(crate) stale: bool,
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
    let mut file = open(path, &metadata, private)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(unsafe_state());
    }
    Ok(Some(bytes))
}

/// Opens the checked path without following links and rechecks the handle.
#[cfg(unix)]
fn open(path: &Path, metadata: &fs::Metadata, private: bool) -> io::Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    if private
        && (metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1)
    {
        return Err(unsafe_state());
    }
    let file = fs::OpenOptions::new()
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
    Ok(file)
}

/// The opened handle, not the earlier path metadata, is checked and read.
#[cfg(windows)]
fn open(path: &Path, _: &fs::Metadata, private: bool) -> io::Result<File> {
    let file = super::dacl::open(path, false)?;
    if private {
        if super::dacl::information(&file)?.nNumberOfLinks != 1 {
            return Err(unsafe_state());
        }
        super::dacl::check(&file)?;
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open(_: &Path, _: &fs::Metadata, _: bool) -> io::Result<File> {
    Err(unsafe_state())
}

/// Repository identity: device and inode, or volume serial number and file
/// index on Windows. A same-path re-clone gets a new identity.
fn identity(path: &Path) -> io::Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path)?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        let info = super::dacl::information(&super::dacl::open(path, true)?)?;
        let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        Ok((info.dwVolumeSerialNumber.into(), index))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
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
    #[cfg(windows)]
    {
        match super::dacl::create_directory(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        super::dacl::check(&super::dacl::open(path, true)?)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(unsafe_state())
    }
}

impl TrustStore {
    // The caller chooses a user location, never a repository-authored path.
    // No writes until publication. Missing user state simply prohibits reuse.
    pub(crate) fn new(base: &Path, root: &Path) -> io::Result<Self> {
        if !cfg!(any(unix, windows))
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

    pub(crate) fn path(&self) -> PathBuf {
        // Encoded native bytes avoid lossy collisions for non-UTF8 Unix roots.
        self.directory.join(format!(
            "{}.json",
            hash(self.root.as_os_str().as_encoded_bytes())
        ))
    }

    fn load(&self) -> io::Result<Option<Record>> {
        // Reject links in any existing state component before opening a record.
        // Only the record directory setup creates must be private: on macOS its
        // parent is turbo's shared config directory, and existing ancestors keep
        // the modes the user's umask gave them (a 0775 `~/.local/share` under
        // Ubuntu's 0002). Another user who can write an ancestor still cannot
        // substitute the owned, private record directory or records below it.
        for path in self.directory.ancestors() {
            match fs::symlink_metadata(path) {
                Ok(metadata) => {
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err(unsafe_state());
                    }
                    #[cfg(target_os = "macos")]
                    super::acl::check(&File::open(path)?, path == self.directory)?;
                    // Windows checks only the record directory's descriptor;
                    // ancestors only need to be real directories. Junctions
                    // are symlinks above; reject every other reparse point.
                    #[cfg(windows)]
                    if path == self.directory {
                        super::dacl::check(&super::dacl::open(path, true)?)?;
                    } else if std::os::windows::fs::MetadataExt::file_attributes(&metadata) & 0x400
                        != 0
                    {
                        return Err(unsafe_state());
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        // Foreign-owned and world-writable ancestors are unsafe,
                        // except root-owned sticky temp directories.
                        let sticky_root = metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
                        if (metadata.uid() != 0 && metadata.uid() != unsafe { libc::geteuid() })
                            || (metadata.mode() & 0o002 != 0 && !sticky_root)
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
        self.inspect_installation(|| store.current().map_err(io::Error::other))
    }

    fn inspect_installation(
        &self,
        inspect: impl FnOnce() -> io::Result<Option<turborepo_tool_install::Current>>,
    ) -> io::Result<Option<Installation>> {
        // Split components: `/` is not a separator under a `\\?\` root.
        let path = self.root.join(".turbo").join("tools").join("manifest.json");
        let Some(before) = read(&path, false)? else {
            return Ok(None);
        };
        let Some(current) = inspect()? else {
            return Ok(None);
        };
        if read(&path, false)?.as_deref() != Some(before.as_slice()) {
            return Err(unsafe_state());
        }
        let (device, inode) = identity(&self.root)?;
        Ok(Some(Installation {
            root: self.root.clone(),
            device,
            inode,
            manifest_sha256: hash(&before),
            // Windows records the PATH spelling that activation uses, because
            // cmd.exe cannot resolve `%~dp0..` under `\\?\`. No-op on Unix.
            bin: dunce::simplified(&current.bin).to_path_buf(),
            tools: current.tools,
        }))
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

    /// Lenient read-only view for the shell hook. Like allows_reuse, lock
    /// drift does not prevent selection; it is reported so the hook can keep
    /// the last-installed tools active with a notice. The current installation
    /// must still equal the recorded one. MUST NOT authorize run or exec,
    /// which stay strict. Never opens a writable Store or creates user state.
    pub(crate) fn recorded(&self) -> io::Result<Option<Recorded>> {
        let Some(record) = self.load()? else {
            return Ok(None);
        };
        let installation = self
            .inspect_installation(|| Store::inspect(&self.root).map_err(io::Error::other))
            .ok()
            .flatten();
        if installation.as_ref() != Some(&record.installation) {
            return Ok(None);
        }
        let lock = read(&self.root.join("turbo.lock"), false)?;
        Ok(Some(Recorded {
            stale: lock.is_none_or(|lock| hash(&lock) != record.lock_sha256),
            bin: record.installation.bin,
        }))
    }

    /// Strict read-only consumer check, unlike setup reuse. The supplied plan
    /// must already validate native declarations and be revalidated before use.
    /// Never opens a writable Store or creates missing user state.
    pub(super) fn authorizes(
        &self,
        plan: &turborepo_setup::activation::ActivationPlan,
    ) -> io::Result<bool> {
        let Some(record) = self.load()? else {
            return Ok(false);
        };
        let Some(lock) = read(&self.root.join("turbo.lock"), false)? else {
            return Ok(false);
        };
        if hash(&lock) != record.lock_sha256 {
            return Ok(false);
        }
        let installation =
            self.inspect_installation(|| Store::inspect(&self.root).map_err(io::Error::other))?;
        Ok(installation.as_ref() == Some(&record.installation)
            && plan.path_prepend() == Some(record.installation.bin.as_path())
            && record.installation.tools == plan.tools()
            && read(&self.root.join("turbo.lock"), false)?.as_deref() == Some(lock.as_slice())
            && self.load()?.is_some_and(|after| {
                after.lock_sha256 == record.lock_sha256 && after.installation == record.installation
            }))
    }

    /// Only call under source/writer/Store guards after the cohort was freshly
    /// staged from verified artifacts or reused from matching external
    /// evidence. Then prunes other roots' records that can never authorize.
    pub(super) fn publish(&self, store: &Store) -> io::Result<()> {
        self.record(store)?;
        // Stale records already never authorize, so failing to remove one must
        // not fail an otherwise successful setup.
        if let Err(error) = self.prune() {
            tracing::debug!("could not prune stale setup records: {error}");
        }
        Ok(())
    }

    /// Remove other roots' records whose root is gone or is now a different
    /// directory (deleted, moved or re-cloned). Reads stay bounded and never
    /// follow links. Unsafe, corrupt or misfiled records are left alone.
    fn prune(&self) -> io::Result<usize> {
        let own = self.path();
        let mut removed = 0;
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        for entry in entries.take(MAX_RECORDS) {
            let path = entry?.path();
            if path == own || path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Ok(Some(bytes)) = read(&path, true) else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<Record>(&bytes) else {
                continue;
            };
            let installation = &record.installation;
            let name = hash(installation.root.as_os_str().as_encoded_bytes());
            if path != self.directory.join(format!("{name}.json")) {
                continue;
            }
            let stale = match fs::metadata(&installation.root) {
                Err(error) => error.kind() == io::ErrorKind::NotFound,
                #[cfg(unix)]
                Ok(metadata) => {
                    use std::os::unix::fs::MetadataExt;
                    (metadata.dev(), metadata.ino()) != (installation.device, installation.inode)
                }
                #[cfg(not(unix))]
                Ok(_) => false,
            };
            // A concurrent setup may have just replaced this record; keep it.
            if stale && read(&path, true)?.as_deref() == Some(bytes.as_slice()) {
                fs::remove_file(&path)?;
                removed += 1;
            }
        }
        if removed > 0 {
            File::open(&self.directory)?.sync_all()?;
        }
        Ok(removed)
    }

    fn record(&self, store: &Store) -> io::Result<()> {
        self.load()?;
        let installation = self.installation(store)?.ok_or_else(unsafe_state)?;
        self.write_record(installation)
    }

    fn write_record(&self, installation: Installation) -> io::Result<()> {
        let Some(lock) = read(&self.root.join("turbo.lock"), false)? else {
            // Ephemeral setup succeeds without creating or retaining locked-run
            // authority. A future committed lock requires setup again.
            return match fs::remove_file(self.path()) {
                Ok(()) => File::open(&self.directory)?.sync_all(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
        };
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
        #[cfg(not(windows))]
        let mut staged = tempfile::NamedTempFile::new_in(&self.directory)?;
        #[cfg(windows)]
        let mut staged =
            tempfile::Builder::new().make_in(&self.directory, super::dacl::create_file)?;
        #[cfg(target_os = "macos")]
        super::acl::check(staged.as_file(), true)?;
        #[cfg(windows)]
        super::dacl::check(staged.as_file())?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged.persist(self.path()).map_err(|e| e.error)?;
        // A read-only Windows directory handle cannot be flushed. A rename
        // lost in a crash only makes the next setup verify artifacts again.
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

#[cfg(all(test, windows))]
#[path = "trust_windows_tests.rs"]
mod windows_tests;
