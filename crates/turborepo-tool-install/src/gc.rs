//! Collection of trees, environments and repository state no repository
//! uses, after a grace period that protects processes still running from a
//! replaced environment. Run, exec and the hook never lock, so the grace
//! period is a bound, not a guarantee: a process that outlives it after its
//! repository switched environments can lose its tools.
//!
//! The grace clock has one writer. `<store>/gc-state.json` maps each
//! unreferenced entry (`envs/<key>`, `trees/<key>`, `repos/<name>`) to when
//! collection first saw it unreferenced. Only collection writes it, under the
//! exclusive lease, so the time is set once, cleared when the entry is
//! referenced again, and never refreshed by setups or runs. A lost or corrupt
//! file restarts every clock, which only delays deletion.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    Error, UserStore, is_sha256,
    store::{EnvKey, lock_file, staging, tree_key},
    sync_directory,
};

/// How long an entry stays after collection first sees it unreferenced.
pub const GRACE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const STATE: &str = "gc-state.json";

/// What one `repos/<name>` directory contributes to collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoState {
    /// The record of a repository that still exists with its recorded
    /// identity. Keeps the directory and its environment.
    Root(EnvKey),
    /// No record, or one whose repository is gone or replaced. The directory
    /// is collected after the grace period; it keeps no environment.
    Dropped,
    /// State collection cannot judge, such as an unsafe or corrupt record.
    /// Left alone; it keeps no environment.
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub entries: usize,
    pub bytes: u64,
}

impl Usage {
    fn add(&mut self, bytes: u64) {
        self.entries += 1;
        self.bytes += bytes;
    }
}

/// What one collection removed and what it kept inside the grace period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Collected {
    pub removed: Usage,
    pub retained: Usage,
}

/// An exclusive hold on `gc.lock`: no setup is publishing or recording while
/// it lives. Removing store entries requires one.
struct ExclusiveLease {
    _lock: File,
}

impl ExclusiveLease {
    /// `Busy` while any setup holds the shared lease: collection never waits
    /// and never makes a setup wait.
    fn try_acquire(store: &Path) -> Result<Acquired, Error> {
        // Without the lock file no setup ever published anything. Creating it
        // would be the only write a lockless offline setup makes.
        let lock = match lock_file(store, false) {
            Ok(lock) => lock,
            Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Acquired::Empty);
            }
            Err(e) => return Err(e),
        };
        match lock.try_lock() {
            Ok(()) => Ok(Acquired::Held(Self { _lock: lock })),
            Err(fs::TryLockError::WouldBlock) => Ok(Acquired::Busy),
            Err(fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

enum Acquired {
    Held(ExclusiveLease),
    Busy,
    /// A store no setup ever published into.
    Empty,
}

impl UserStore {
    /// Removes every tree, environment and repository directory that has
    /// been unreferenced for [`GRACE`] at `now`, plus all staging. Roots are
    /// the `repos/<name>` directories `classify` judges [`RepoState::Root`];
    /// their environments and the trees those environments seal are live.
    /// `None` when a setup holds the store: this round is skipped. Failing
    /// to read a root, or the seal of an environment that stays, stops
    /// collection before it removes anything that could still be needed:
    /// it never acts on a partial view of the references.
    pub fn collect(
        &self,
        now: SystemTime,
        mut classify: impl FnMut(&Path) -> RepoState,
    ) -> Result<Option<Collected>, Error> {
        let lease = match ExclusiveLease::try_acquire(&self.root)? {
            Acquired::Held(lease) => lease,
            Acquired::Busy => return Ok(None),
            Acquired::Empty => return Ok(Some(Collected::default())),
        };
        // No setup holds the store, so all staging is abandoned, including
        // trash from a collection that crashed after moving an entry aside.
        for entry in read_dir(&self.tmp())? {
            remove(&entry?.path());
        }

        let mut live_envs = BTreeSet::new();
        let mut candidates = Vec::new();
        for (name, path) in entries(&self.repos())? {
            match classify(&path) {
                RepoState::Root(env) => {
                    live_envs.insert(env);
                }
                RepoState::Dropped => candidates.push(format!("repos/{name}")),
                RepoState::Unknown => {}
            }
        }
        let mut live_trees = BTreeSet::new();
        for env in &live_envs {
            live_trees.extend(self.env_trees(env)?);
        }
        let mut envs = BTreeSet::new();
        for (name, _) in entries(&self.root.join("envs"))? {
            let env = EnvKey::try_from(name.clone())?;
            if !live_envs.contains(&env) {
                candidates.push(format!("envs/{name}"));
            }
            envs.insert(env);
        }
        for (name, _) in entries(&self.root.join("trees"))? {
            if !live_trees.contains(&name) {
                candidates.push(format!("trees/{name}"));
            }
        }

        let seen = load_state(&self.root);
        let now_secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut state = BTreeMap::new();
        let mut expired = BTreeSet::new();
        for entry in &candidates {
            let since = seen.get(entry).copied().unwrap_or(now_secs);
            state.insert(entry.clone(), since);
            // A clock that went backwards never makes anything expire.
            if now_secs
                .checked_sub(since)
                .is_some_and(|age| age >= GRACE.as_secs())
            {
                expired.insert(entry.clone());
            }
        }

        let mut collected = Collected::default();
        let mut removed = BTreeSet::new();
        let (env_entries, rest): (Vec<_>, Vec<_>) =
            candidates.iter().partition(|e| e.starts_with("envs/"));
        for entry in env_entries {
            let expired = expired.contains(entry);
            self.settle(&lease, entry, expired, &mut collected, &mut removed);
        }
        // A tree whose own clock expired stays while any environment that
        // survived this pass still names it.
        let mut kept_trees = live_trees;
        for env in envs {
            if !removed.contains(&format!("envs/{env}")) {
                kept_trees.extend(self.env_trees(&env)?);
            }
        }
        for entry in rest {
            let protected = entry
                .strip_prefix("trees/")
                .is_some_and(|tree| kept_trees.contains(tree));
            let expired = expired.contains(entry) && !protected;
            self.settle(&lease, entry, expired, &mut collected, &mut removed);
        }

        state.retain(|entry, _| !removed.contains(entry));
        if state != seen {
            save_state(&lease, &self.root, &self.tmp(), &state)?;
        }
        Ok(Some(collected))
    }

    /// Removes an expired candidate, or counts it as retained, including when
    /// removal fails (a Windows executable still running).
    fn settle(
        &self,
        lease: &ExclusiveLease,
        entry: &str,
        expired: bool,
        collected: &mut Collected,
        removed: &mut BTreeSet<String>,
    ) {
        let path = self.root.join(entry);
        let bytes = disk_usage(&path);
        if expired && discard(lease, &path, &self.tmp()).is_ok() {
            collected.removed.add(bytes);
            removed.insert(entry.to_owned());
        } else {
            collected.retained.add(bytes);
        }
    }

    /// The trees an environment's seal names; none for a missing or invalid
    /// seal, which no activation accepts either.
    fn env_trees(&self, env: &EnvKey) -> Result<Vec<String>, Error> {
        Ok(self
            .env_seal(&self.env_dir(env), env)?
            .map(|selection| selection.tools().iter().map(tree_key).collect())
            .unwrap_or_default())
    }
}

fn read_dir(dir: &Path) -> Result<Box<dyn Iterator<Item = io::Result<fs::DirEntry>>>, Error> {
    match fs::read_dir(dir) {
        Ok(entries) => Ok(Box::new(entries)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Box::new(std::iter::empty())),
        Err(e) => Err(e.into()),
    }
}

/// The real, key-named directories in `dir`. Anything else there is not the
/// store's to judge.
fn entries(dir: &Path) -> Result<Vec<(String, std::path::PathBuf)>, Error> {
    let mut found = Vec::new();
    for entry in read_dir(dir)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if is_sha256(&name) && entry.file_type()?.is_dir() {
            found.push((name, entry.path()));
        }
    }
    Ok(found)
}

fn load_state(root: &Path) -> BTreeMap<String, u64> {
    fs::read(root.join(STATE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_state(
    _lease: &ExclusiveLease,
    root: &Path,
    tmp: &Path,
    state: &BTreeMap<String, u64>,
) -> Result<(), Error> {
    let staged = tempfile::NamedTempFile::new_in(tmp)?;
    serde_json::to_writer_pretty(staged.as_file(), state)?;
    staged.as_file().sync_all()?;
    staged.persist(root.join(STATE)).map_err(|e| e.error)?;
    sync_directory(root)
}

/// Moves `path` into private trash with one same-device rename, then removes
/// the trash. A crash leaves only trash, which the next collection removes.
/// The rename moves a link itself, never its target.
fn discard(_lease: &ExclusiveLease, path: &Path, tmp: &Path) -> Result<(), Error> {
    let trash = staging(tmp, "trash-")?.keep();
    if let Err(e) = fs::rename(path, trash.join("entry")) {
        let _ = fs::remove_dir(&trash);
        return Err(e.into());
    }
    remove(&trash);
    Ok(())
}

/// Best effort: whatever is still held open is retried by the next
/// collection.
fn remove(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.is_dir() {
        let _ = fs::remove_file(path);
        return;
    }
    if fs::remove_dir_all(path).is_ok() {
        return;
    }
    // Store files are read-only but directories are not, except in trees an
    // adapter extracted with read-only directories.
    #[cfg(unix)]
    {
        make_directories_writable(path);
        let _ = fs::remove_dir_all(path);
    }
}

#[cfg(unix)]
fn make_directories_writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = fs::symlink_metadata(dir) else {
        return;
    };
    if !metadata.is_dir() {
        return;
    }
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        make_directories_writable(&entry.path());
    }
}

/// Bytes of the regular files under `path`, never following links.
fn disk_usage(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| disk_usage(&entry.path()))
        .sum()
}
