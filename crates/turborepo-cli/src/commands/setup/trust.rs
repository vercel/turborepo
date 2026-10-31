//! The repository's record in the user's tool store: the user-owned evidence
//! that binds one repository, by directory identity and turbo.lock digest, to
//! the environment setup installed for it. Run consumers must additionally
//! gate managed mode, validate declarations/lock and revalidate immediately
//! before use. Unix privacy uses owner, mode and macOS ACL checks; Windows
//! uses owner-only DACLs read from each opened handle (see `dacl`).

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use turborepo_tool_install::{Activation, EnvKey, RepoState, UserStore};

const LIMIT: u64 = 1024 * 1024;

const SCHEMA: u32 = 2;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    root: PathBuf,
    /// Unix device and inode, or Windows volume serial number and file index.
    device: u64,
    inode: u64,
    /// The turbo.lock digest. Absent only for a `--no-lock` clone, which its
    /// clone-local selection identifies instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lock_sha256: Option<String>,
    /// `turbo setup --no-lock`'s canonical selection, in place of turbo.lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<String>,
    env: EnvKey,
}

impl Record {
    /// The resolution this record was published for: the current turbo.lock
    /// digest, or no lock at all for a clone-local selection.
    fn resolves(&self, lock: Option<&[u8]>) -> bool {
        lock.map(hash) == self.lock_sha256
    }
}

pub(crate) struct TrustStore {
    store: UserStore,
    /// This repository's directory in the user store.
    directory: PathBuf,
    root: PathBuf,
}

/// Setup-recorded tools for the interactive shell hook only.
pub(crate) struct Recorded {
    pub(crate) activation: Activation,
    /// turbo.lock changed since setup recorded this environment, including a
    /// lock added after `turbo setup --no-lock`.
    pub(crate) stale: bool,
}

/// Whether `root` has a `turbo setup --no-lock` record in the tool store
/// (`store`, else the user's): a routing hint, never authorization.
/// The fixture's tool store, else the user's. Unit tests must name a store:
/// the user's is never theirs to read or write.
pub(crate) fn user_store(
    fixture: Option<&Path>,
    root: &Path,
) -> Result<UserStore, turborepo_tool_install::Error> {
    match fixture {
        Some(path) => UserStore::at(path, root),
        #[cfg(test)]
        None => Err(turborepo_tool_install::Error::NoStore),
        #[cfg(not(test))]
        None => UserStore::locate(root),
    }
}

pub(crate) fn records_clone_local(root: &Path, store: Option<&Path>) -> io::Result<bool> {
    let root = root.canonicalize()?;
    let store = user_store(store, &root).map_err(io::Error::other)?;
    Ok(TrustStore::new(&store, &root)?
        .clone_local_selection()?
        .is_some())
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
    // `UserStore` located the store outside the repository. No writes until
    // publication. Missing user state simply prohibits reuse.
    pub(crate) fn new(store: &UserStore, root: &Path) -> io::Result<Self> {
        let root = root.canonicalize()?;
        Ok(Self {
            directory: store.repo(&root),
            store: store.clone(),
            root,
        })
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.directory.join("record.json")
    }

    pub(crate) fn store(&self) -> &UserStore {
        &self.store
    }

    /// The repository's private `CARGO_HOME`, beside its record.
    pub(super) fn cargo_home(&self) -> PathBuf {
        self.store.cargo_home(&self.root)
    }

    fn load(&self) -> io::Result<Option<Record>> {
        // Reject links in any existing state component before opening a record.
        // Only the record directory setup creates must be private: existing
        // ancestors keep the modes the user's umask gave them (a 0775
        // `~/.local/share` under Ubuntu's 0002). Another user who can write an
        // ancestor still cannot substitute the owned, private record directory
        // or records below it.
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
        // A lockless record must name the clone-local selection it trusts.
        Ok(serde_json::from_slice::<Record>(&bytes).ok().filter(|r| {
            r.schema == SCHEMA
                && r.root == self.root
                && (r.lock_sha256.is_some() != r.selection.is_some())
        }))
    }

    pub(super) fn preflight(&self) -> io::Result<()> {
        self.load().map(|_| ())
    }

    /// Whether the record still names this exact directory: a re-clone or
    /// move at the same path gets a new identity.
    fn same_directory(&self, record: &Record) -> io::Result<bool> {
        Ok(identity(&self.root)? == (record.device, record.inode))
    }

    /// The clone-local selection `turbo setup --no-lock` recorded, if this
    /// repository has one.
    pub(crate) fn clone_local_selection(&self) -> io::Result<Option<Vec<u8>>> {
        Ok(self
            .load()?
            .and_then(|record| record.selection)
            .map(String::into_bytes))
    }

    /// The environment this repository's record names, whatever turbo.lock
    /// says now.
    pub(super) fn recorded_env(&self) -> io::Result<Option<EnvKey>> {
        Ok(self.load()?.map(|record| record.env))
    }

    /// Lenient read-only view for the shell hook: lock drift does not prevent
    /// activation; it is reported so the hook can keep the last-installed
    /// tools active with a notice. MUST NOT authorize run or exec, which stay
    /// strict. Reads the record and the environment's seal, never a tree.
    pub(crate) fn recorded(&self) -> io::Result<Option<Recorded>> {
        let Some(record) = self.load()? else {
            return Ok(None);
        };
        if !self.same_directory(&record)? {
            return Ok(None);
        }
        let Ok(activation) = self.store.activate(&self.root, &record.env) else {
            return Ok(None);
        };
        let lock = read(&self.root.join("turbo.lock"), false)?;
        Ok(Some(Recorded {
            stale: !record.resolves(lock.as_deref()),
            activation,
        }))
    }

    /// Strict read-only consumer check. The supplied plan must already
    /// validate native declarations and be revalidated before use. A committed
    /// turbo.lock authorizes only a record for its exact digest; without one,
    /// only a `--no-lock` record for the planned clone-local selection. The
    /// record must name this directory and exactly the plan's environment.
    /// Never writes and never reads a tree.
    pub(super) fn authorize(
        &self,
        plan: &turborepo_setup::activation::ActivationPlan,
    ) -> Result<Option<Activation>, super::authorization::Error> {
        let Some(record) = self.load()? else {
            return Ok(None);
        };
        let lock = read(&self.root.join("turbo.lock"), false)?;
        if !record.resolves(lock.as_deref())
            || &record.env != plan.selection().key()
            || !self.same_directory(&record)?
        {
            return Ok(None);
        }
        let activation = self.store.activate(&self.root, &record.env)?;
        // Nothing changed underneath the check.
        if read(&self.root.join("turbo.lock"), false)? != lock
            || self.load()?.as_ref() != Some(&record)
        {
            return Ok(None);
        }
        Ok(Some(activation))
    }

    /// Binds this repository to `env`, which setup just converged, under the
    /// source and writer guards. `selection` is a `--no-lock` clone's
    /// selection; with neither it nor turbo.lock, no record remains. Records
    /// that can never authorize again are collection's to remove, after its
    /// grace period ([`repo_state`]).
    pub(crate) fn publish(&self, env: &EnvKey, selection: Option<&[u8]>) -> io::Result<()> {
        self.write_record(env, selection)
    }

    fn write_record(&self, env: &EnvKey, selection: Option<&[u8]>) -> io::Result<()> {
        self.load()?;
        let lock = read(&self.root.join("turbo.lock"), false)?;
        // A committed lock is the resolution; a clone-local selection only
        // stands in for a missing one. Without either, setup succeeds without
        // creating or retaining run authority.
        let selection = match (&lock, selection) {
            (Some(_), _) => None,
            (None, Some(selection)) => {
                Some(String::from_utf8(selection.to_vec()).map_err(|_| unsafe_state())?)
            }
            (None, None) => {
                return match fs::remove_file(self.path()) {
                    Ok(()) => sync_directory(&self.directory),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error),
                };
            }
        };
        let (device, inode) = identity(&self.root)?;
        let bytes = serde_json::to_vec(&Record {
            schema: SCHEMA,
            root: self.root.clone(),
            device,
            inode,
            lock_sha256: lock.as_deref().map(hash),
            selection,
            env: env.clone(),
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
        sync_directory(&self.directory)
    }
}

/// Whether the repository state `directory` keeps its environment: only a
/// record filed under its own root, for a root that still has the recorded
/// identity. A deleted, moved or re-cloned repository's state is dropped and
/// collected after the grace period. Unsafe, corrupt or misfiled records are
/// left alone and keep nothing. Never follows links.
pub(crate) fn repo_state(store: &UserStore, directory: &Path) -> RepoState {
    let bytes = match read(&directory.join("record.json"), true) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return RepoState::Dropped,
        Err(_) => return RepoState::Unknown,
    };
    let Ok(record) = serde_json::from_slice::<Record>(&bytes) else {
        return RepoState::Unknown;
    };
    if directory != store.repo(&record.root) {
        return RepoState::Unknown;
    }
    match identity(&record.root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => RepoState::Dropped,
        Ok(found) if found != (record.device, record.inode) => RepoState::Dropped,
        // An unreadable root may still exist: keep what it names.
        _ => RepoState::Root(record.env),
    }
}

/// A read-only Windows directory handle cannot be flushed. A rename or removal
/// lost in a crash only makes the next setup verify artifacts again.
fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(all(test, windows))]
#[path = "trust_windows_tests.rs"]
mod windows_tests;
