use std::{backtrace::Backtrace, collections::HashMap, io::BufWriter, time::UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, IntoUnix, RelativeUnixPath};

use crate::CacheError;

/// Version of what a manifest records. Only a manifest of this version
/// records symlinks and directories as well as regular files (version 0
/// recorded no symlinks), so only it can vouch that a whole restore is
/// unnecessary. Older manifests
/// still describe their regular files correctly, so the per-file skip during
/// a restore keeps using them.
const MANIFEST_VERSION: u32 = 1;

/// Records the size, mtime, and mode of each file, every directory, and the
/// target of each symlink in a cache archive's outputs, as they were on disk
/// when the archive was saved or restored. Used to skip redundant writes on
/// subsequent restores of the same hash, and to skip the restore entirely
/// when every recorded entry still matches.
#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreManifest {
    #[serde(default)]
    version: u32,
    /// symlink path (relative to anchor) -> link target, in unix form, for
    /// each entry in `files` with `is_symlink` set. Readers that predate
    /// symlink entries see those as empty files that never match, so they
    /// never treat a symlink as unchanged.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    link_targets: HashMap<String, String>,
    /// path (relative to anchor) -> (size_bytes, mtime_nanos, mode)
    pub files: HashMap<String, FileEntry>,
    /// Insertion-order list of paths so validate_all can return files in
    /// the same order the archive was originally built.
    #[serde(default)]
    pub order: Vec<String>,
}

/// Returned by [`RestoreManifest::read_at_most`] for a manifest over the
/// caller's size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestTooLarge;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub size: u64,
    pub mtime_nanos: i128,
    pub mode: u32,
    #[serde(default)]
    pub is_dir: bool,
    /// The target is in `RestoreManifest::link_targets`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_symlink: bool,
}

impl Default for RestoreManifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            link_targets: HashMap::new(),
            files: HashMap::new(),
            order: Vec::new(),
        }
    }
}

impl RestoreManifest {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records every entry in `paths` (relative to `anchor`) as it currently
    /// is on disk: regular files, directories, and symlinks. Used when an
    /// already-built archive is installed for outputs on disk; saving outputs
    /// records each entry as it archives it, and a restore as it writes it.
    pub fn from_disk(anchor: &AbsoluteSystemPath, paths: &[AnchoredSystemPathBuf]) -> Self {
        let mut manifest = Self::new();
        for path in paths {
            let disk_path = anchor.resolve(path);
            let key = path.to_unix().into_inner();
            let Ok(meta) = disk_path.symlink_metadata() else {
                continue;
            };
            if meta.is_file() {
                let _ = manifest.record_file(key, &disk_path);
            } else if meta.is_dir() {
                manifest.record_dir(key);
            } else if meta.is_symlink()
                && let Ok(target) = disk_path.read_link()
            {
                manifest.record_symlink(key, target.as_str());
            }
        }
        manifest
    }

    /// Check if a file on disk matches the manifest entry.
    /// Returns true only if the file exists and has the expected size,
    /// mtime, and permissions matching what we recorded when we last
    /// wrote it.
    pub fn file_matches(&self, path: &str, disk_path: &AbsoluteSystemPath) -> bool {
        let Some(expected) = self.files.get(path) else {
            return false;
        };

        let Ok(meta) = disk_path.symlink_metadata() else {
            return false;
        };

        if expected.is_symlink {
            // Compare the targets as strings: path equality ignores a
            // trailing `/`, which turns a link to a file into a broken one.
            return meta.is_symlink()
                && self.link_targets.get(path).is_some_and(|expected_target| {
                    disk_path.read_link().is_ok_and(|target| {
                        target.as_str().into_unix().as_str() == expected_target.as_str()
                    })
                });
        }

        if expected.is_dir {
            return meta.is_dir();
        }

        if !meta.is_file() {
            return false;
        }

        if meta.len() != expected.size {
            return false;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (meta.mode() & 0o777) != expected.mode {
                return false;
            }
        }

        let Ok(modified) = meta.modified() else {
            return false;
        };
        let Ok(duration) = modified.duration_since(UNIX_EPOCH) else {
            return false;
        };

        duration.as_nanos() as i128 == expected.mtime_nanos
    }

    /// Record a file that was just written to disk.
    pub fn record_file(
        &mut self,
        path: String,
        disk_path: &AbsoluteSystemPath,
    ) -> Result<(), CacheError> {
        self.record_file_metadata(path, &disk_path.symlink_metadata()?)
    }

    /// Record a file that was just written to disk, using metadata already
    /// obtained for it (e.g. from the open file handle after the last write).
    pub fn record_file_metadata(
        &mut self,
        path: String,
        meta: &std::fs::Metadata,
    ) -> Result<(), CacheError> {
        let mtime_nanos = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| CacheError::InvalidManifest(e.to_string()))?
            .as_nanos() as i128;

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::MetadataExt;
            meta.mode() & 0o777
        };
        #[cfg(not(unix))]
        let mode = 0o644;

        self.order.push(path.clone());
        self.files.insert(
            path,
            FileEntry {
                size: meta.len(),
                mtime_nanos,
                mode,
                is_dir: false,
                is_symlink: false,
            },
        );
        Ok(())
    }

    /// Record a directory entry in the manifest.
    pub fn record_dir(&mut self, path: String) {
        self.order.push(path.clone());
        self.files.insert(
            path,
            FileEntry {
                size: 0,
                mtime_nanos: 0,
                mode: 0,
                is_dir: true,
                is_symlink: false,
            },
        );
    }

    /// Record a symlink entry and its target in the manifest.
    pub fn record_symlink(&mut self, path: String, target: &str) {
        self.link_targets
            .insert(path.clone(), target.into_unix().into_string());
        self.order.push(path.clone());
        self.files.insert(
            path,
            FileEntry {
                size: 0,
                mtime_nanos: 0,
                mode: 0,
                is_dir: false,
                is_symlink: true,
            },
        );
    }

    /// Check every file in the manifest against disk. If ALL match,
    /// return the list of file paths (suitable for returning from fetch
    /// without opening the tar). Returns None if any file is stale, or if
    /// the manifest predates recording every kind of entry.
    pub fn validate_all(
        &self,
        anchor: &AbsoluteSystemPath,
    ) -> Option<Vec<turbopath::AnchoredSystemPathBuf>> {
        if self.version != MANIFEST_VERSION {
            return None;
        }
        // Use the order vec when present so the returned list matches
        // the original archive order. Fall back to HashMap keys for
        // manifests written before order tracking was added.
        let keys: Vec<&str> = if self.order.len() == self.files.len() {
            self.order.iter().map(|s| s.as_str()).collect()
        } else {
            self.files.keys().map(|s| s.as_str()).collect()
        };

        let mut paths = Vec::with_capacity(keys.len());
        for rel_path in keys {
            let Ok(anchored) = RelativeUnixPath::new(rel_path) else {
                return None;
            };
            let anchored = anchored.to_anchored_system_path_buf();
            let disk_path = anchor.resolve(&anchored);
            if !self.file_matches(rel_path, &disk_path) {
                return None;
            }
            paths.push(anchored);
        }
        Some(paths)
    }

    pub fn read(path: &AbsoluteSystemPath) -> Option<Self> {
        let contents = std::fs::read_to_string(path.as_path()).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// Like [`Self::read`], but returns `Err(ManifestTooLarge)` without
    /// reading the file when it is larger than `max_bytes`.
    pub fn read_at_most(
        path: &AbsoluteSystemPath,
        max_bytes: u64,
    ) -> Result<Option<Self>, ManifestTooLarge> {
        let Ok(mut file) = std::fs::File::open(path.as_path()) else {
            return Ok(None);
        };
        if file.metadata().is_ok_and(|meta| meta.len() > max_bytes) {
            return Err(ManifestTooLarge);
        }
        let mut contents = String::new();
        if std::io::Read::read_to_string(&mut file, &mut contents).is_err() {
            return Ok(None);
        }
        Ok(serde_json::from_str(&contents).ok())
    }

    pub fn write_atomic(&self, path: &AbsoluteSystemPath) -> Result<(), CacheError> {
        let Some(parent) = path.parent() else {
            return Err(CacheError::InvalidFilePath(
                path.to_string(),
                Backtrace::capture(),
            ));
        };
        let tmp_path =
            parent.join_component(&format!("{}.tmp", path.file_name().unwrap_or("manifest")));
        let file = std::fs::File::create(tmp_path.as_path())?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, self)
            .map_err(|e| CacheError::InvalidManifest(e.to_string()))?;
        std::fs::rename(tmp_path.as_path(), path.as_path())?;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

    use super::RestoreManifest;

    #[test]
    fn symlink_matches_only_its_exact_target() -> Result<()> {
        let repo_root = tempdir()?;
        let anchor = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        anchor.join_component("a.txt").create_with_contents("a")?;
        let link = anchor.join_component("link");
        link.symlink_to_file("a.txt")?;
        let manifest =
            RestoreManifest::from_disk(anchor, &[AnchoredSystemPathBuf::from_raw("link")?]);
        assert!(manifest.file_matches("link", &link));

        // `a.txt/` names the same path but is a broken link (ENOTDIR).
        link.remove_file()?;
        link.symlink_to_file("a.txt/")?;
        assert!(!manifest.file_matches("link", &link));
        assert!(manifest.validate_all(anchor).is_none());
        Ok(())
    }
}
