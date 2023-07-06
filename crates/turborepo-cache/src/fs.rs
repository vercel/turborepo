use std::backtrace::Backtrace;

use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
    cache_archive::{CacheReader, CacheWriter},
    CacheError, CacheSource, ItemStatus,
};

struct FSCache {
    cache_directory: AbsoluteSystemPathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct CacheMetadata {
    hash: String,
    duration: u32,
}

impl CacheMetadata {
    fn read(path: &AbsoluteSystemPath) -> Result<CacheMetadata, CacheError> {
        serde_json::from_reader(path.open()?)
            .map_err(|e| CacheError::InvalidMetadata(e, Backtrace::capture()))
    }
}

impl FSCache {
    fn resolve_cache_dir(
        repo_root: &AbsoluteSystemPath,
        override_dir: Option<&str>,
    ) -> AbsoluteSystemPathBuf {
        if let Some(override_dir) = override_dir {
            AbsoluteSystemPathBuf::from_unknown(repo_root, override_dir)
        } else {
            repo_root.join_components(&["node_modules", ".cache", "turbo"])
        }
    }

    pub fn new(
        override_dir: Option<&str>,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<Self, CacheError> {
        let cache_directory = Self::resolve_cache_dir(repo_root, override_dir);
        cache_directory.create_dir_all()?;

        Ok(FSCache { cache_directory })
    }

    pub fn fetch(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
    ) -> Result<(ItemStatus, Vec<AnchoredSystemPathBuf>), CacheError> {
        let uncompressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar", hash));
        let compressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar.zst", hash));

        let cache_path = if uncompressed_cache_path.exists() {
            uncompressed_cache_path
        } else if compressed_cache_path.exists() {
            compressed_cache_path
        } else {
            return Ok((ItemStatus::Miss, vec![]));
        };

        let mut cache_reader = CacheReader::open(&cache_path)?;

        let restored_files = cache_reader.restore(anchor)?;

        let meta = CacheMetadata::read(
            &self
                .cache_directory
                .join_component(&format!("{}-meta.json", hash)),
        )?;

        Ok((
            ItemStatus::Hit {
                time_saved: meta.duration,
                source: CacheSource::Local,
            },
            restored_files,
        ))
    }

    fn exists(&self, hash: &str) -> Result<ItemStatus, CacheError> {
        let uncompressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar", hash));
        let compressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar.zst", hash));

        if !uncompressed_cache_path.exists() && !compressed_cache_path.exists() {
            return Ok(ItemStatus::Miss);
        }

        let duration = CacheMetadata::read(
            &self
                .cache_directory
                .join_component(&format!("{}-meta.json", hash)),
        )
        .map(|meta| meta.duration)
        .unwrap_or(0);

        Ok(ItemStatus::Hit {
            time_saved: duration,
            source: CacheSource::Local,
        })
    }

    fn put(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
        duration: u32,
        files: Vec<AnchoredSystemPathBuf>,
    ) -> Result<(), CacheError> {
        let cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar.zst", hash));

        let mut cache_item = CacheWriter::create(&cache_path)?;

        for file in files {
            cache_item.add_file(anchor, &file)?;
        }

        let meta_path = self
            .cache_directory
            .join_component(&format!("{}-meta.json", hash));

        let meta = CacheMetadata {
            hash: hash.to_string(),
            duration,
        };

        serde_json::to_writer(meta_path.open()?, &meta)
            .map_err(|e| CacheError::InvalidMetadata(e, Backtrace::capture()))?;

        Ok(())
    }
}
