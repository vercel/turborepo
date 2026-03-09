use std::{collections::HashMap, io::BufWriter, time::UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPath;

use crate::CacheError;

/// Records the size, mtime, and mode of each file written during a cache
/// restore. Used to skip redundant writes on subsequent restores of the
/// same hash.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RestoreManifest {
    /// path (relative to anchor) -> (size_bytes, mtime_nanos, mode)
    pub files: HashMap<String, FileEntry>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub size: u64,
    pub mtime_nanos: i128,
    pub mode: u32,
}

impl RestoreManifest {
    pub fn new() -> Self {
        Self::default()
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

        if !meta.is_file() {
            return false;
        }

        if meta.len() != expected.size {
            return false;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (meta.mode() & 0o7777) != expected.mode {
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
        let meta = disk_path.symlink_metadata()?;
        let mtime_nanos = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| CacheError::InvalidManifest(e.to_string()))?
            .as_nanos() as i128;

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::MetadataExt;
            meta.mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = 0o644;

        self.files.insert(
            path,
            FileEntry {
                size: meta.len(),
                mtime_nanos,
                mode,
            },
        );
        Ok(())
    }

    /// Check every file in the manifest against disk. If ALL match,
    /// return the list of file paths (suitable for returning from fetch
    /// without opening the tar). Returns None if any file is stale.
    pub fn validate_all(
        &self,
        anchor: &AbsoluteSystemPath,
    ) -> Option<Vec<turbopath::AnchoredSystemPathBuf>> {
        let mut paths = Vec::with_capacity(self.files.len());
        for rel_path in self.files.keys() {
            let Ok(anchored) = turbopath::AnchoredSystemPathBuf::from_raw(rel_path) else {
                return None;
            };
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

    pub fn write_atomic(&self, path: &AbsoluteSystemPath) -> Result<(), CacheError> {
        let tmp_path = path
            .parent()
            .unwrap()
            .join_component(&format!("{}.tmp", path.file_name().unwrap_or("manifest")));
        let file = std::fs::File::create(tmp_path.as_path())?;
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, self)
            .map_err(|e| CacheError::InvalidManifest(e.to_string()))?;
        std::fs::rename(tmp_path.as_path(), path.as_path())?;
        Ok(())
    }
}
