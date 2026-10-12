//! Shared, content-addressed cache of verified setup downloads: archive bytes
//! only, never extracted trees, shims, or anything on a PATH. Every repository
//! still extracts into its own `.turbo/tools` generation, so activation stays
//! repository-scoped. Entries are keyed by the lock's artifact identity
//! (platform key plus trusted SHA-256), and every read re-verifies that digest.
//! Writers publish complete verified files by atomic rename, so concurrent
//! writers and interrupted downloads never expose partial bytes. Hits and lock
//! references record their last use for age-based pruning. No network or
//! cross-process locking.

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

use turborepo_download::{ExpectedSha256, VerifiedArtifact};
pub use turborepo_tool_install::DiskUse;

use crate::lock::Platform;

#[derive(Clone, Debug)]
pub struct ArtifactCache {
    root: PathBuf,
}

impl ArtifactCache {
    /// `base` is a caller-selected user cache directory, never a repository
    /// path or repository-authored setting. Construction has no filesystem
    /// effects; nothing is created until the first verified `put`.
    pub fn new(base: &Path) -> Option<Self> {
        if !base.is_absolute()
            || base
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return None;
        }
        Some(Self {
            root: base.join("turborepo").join("setup-artifacts").join("v1"),
        })
    }

    /// A verified hit, or `None`. Missing, unsafe, oversized, and corrupt
    /// entries are all misses: the caller downloads (or reports) instead.
    pub fn get(
        &self,
        platform: Platform,
        sha256: &str,
        max_bytes: usize,
    ) -> Option<VerifiedArtifact> {
        let digest = ExpectedSha256::from_hex(sha256).ok()?;
        let directory = self.directory(platform, sha256)?;
        self.check(&directory).ok()?;
        let mut file = open_entry(&directory.join(sha256)).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > max_bytes as u64 {
            return None;
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > max_bytes {
            return None;
        }
        let artifact = VerifiedArtifact::verify_bytes(bytes, digest).ok()?;
        // Last use, from any repository, is the pruning signal. Best effort.
        let _ = file.set_modified(SystemTime::now());
        Some(artifact)
    }

    /// Refresh the last use of an entry the caller's lock references without
    /// reading it, so a clone that reuses its installs still keeps the bytes
    /// it would need to reinstall. Best effort.
    pub fn mark_used(&self, platform: Platform, sha256: &str) {
        let Some(directory) = self.directory(platform, sha256) else {
            return;
        };
        if self.check(&directory).is_ok()
            && let Ok(file) = open_entry(&directory.join(sha256))
            && file.metadata().is_ok_and(|metadata| metadata.is_file())
        {
            let _ = file.set_modified(SystemTime::now());
        }
    }

    /// Remove entries no repository has used within `unused_for`, plus stale
    /// partial writes. Extracted installs never depend on cache entries, so a
    /// pruned entry only costs a later verified download. Unrecognized names
    /// and links are never touched.
    pub fn prune_unused(&self, unused_for: Duration) -> io::Result<DiskUse> {
        let cutoff = SystemTime::now()
            .checked_sub(unused_for)
            .ok_or_else(|| io::Error::other("invalid setup download cache age"))?;
        let mut removed = DiskUse::default();
        for path in self.entries()? {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_file() && metadata.modified()? < cutoff {
                fs::remove_file(&path)?;
                removed.entries += 1;
                removed.bytes += metadata.len();
            }
        }
        Ok(removed)
    }

    /// Apparent size of cached entries and partials, shared by every clone.
    pub fn disk_use(&self) -> io::Result<DiskUse> {
        let mut usage = DiskUse::default();
        for path in self.entries()? {
            usage.entries += 1;
            usage.bytes += fs::symlink_metadata(path)?.len();
        }
        Ok(usage)
    }

    // Digest entries and partials in real platform directories only.
    fn entries(&self) -> io::Result<Vec<PathBuf>> {
        let platforms = match fs::read_dir(&self.root) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            result => result?,
        };
        let mut entries = Vec::new();
        for platform in platforms {
            let platform = platform?.path();
            if self.check(&platform).is_err() {
                continue;
            }
            for entry in fs::read_dir(platform)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let digest = name.len() == 64
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if entry.file_type()?.is_file() && (digest || name.starts_with(".partial-")) {
                    entries.push(entry.path());
                }
            }
        }
        Ok(entries)
    }

    /// Publish verified bytes under their own SHA-256. Concurrent writers of
    /// the same artifact each rename a complete private file over identical
    /// content; a corrupt entry is replaced rather than trusted.
    pub fn put(&self, platform: Platform, artifact: &VerifiedArtifact) -> io::Result<()> {
        let sha256 = artifact.sha256_hex();
        let directory = self
            .directory(platform, &sha256)
            .ok_or_else(|| io::Error::other("invalid setup download cache key"))?;
        create_private(&directory)?;
        self.check(&directory)?;
        // Partials share the entry directory (same filesystem for rename) and
        // can never match a digest-only entry name.
        let mut file = tempfile::Builder::new()
            .prefix(".partial-")
            .tempfile_in(&directory)?;
        file.write_all(artifact.as_bytes())?;
        file.as_file().sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o444))?;
        }
        file.persist(directory.join(&sha256))
            .map_err(|error| error.error)?;
        Ok(())
    }

    fn directory(&self, platform: Platform, sha256: &str) -> Option<PathBuf> {
        let serde_json::Value::String(platform) = serde_json::to_value(platform).ok()? else {
            return None;
        };
        let lowercase_hex = sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        (sha256.len() == 64 && lowercase_hex).then(|| self.root.join(platform))
    }

    // Only components this cache owns must be real directories; the caller's
    // base may legitimately sit behind a symlink (for example macOS `/var`).
    fn check(&self, directory: &Path) -> io::Result<()> {
        let base = self.root.ancestors().nth(3);
        for path in directory.ancestors() {
            if Some(path) == base {
                return Ok(());
            }
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(io::Error::other("unsafe setup download cache directory"));
            }
        }
        Err(io::Error::other("unsafe setup download cache directory"))
    }
}

#[cfg(unix)]
fn open_entry(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    // Never follow a planted link; never block on a FIFO.
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_entry(_: &Path) -> io::Result<File> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

#[cfg(not(unix))]
fn create_private(_: &Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(all(test, unix))]
mod tests;
