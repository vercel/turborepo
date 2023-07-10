use std::{backtrace::Backtrace, fs::OpenOptions};

use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::{
    cache_archive::{CacheReader, CacheWriter},
    CacheError, CacheResponse, CacheSource,
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
    ) -> Result<(CacheResponse, Vec<AnchoredSystemPathBuf>), CacheError> {
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
            return Err(CacheError::CacheMiss);
        };

        let mut cache_reader = CacheReader::open(&cache_path)?;

        let restored_files = cache_reader.restore(anchor)?;

        let meta = CacheMetadata::read(
            &self
                .cache_directory
                .join_component(&format!("{}-meta.json", hash)),
        )?;

        Ok((
            CacheResponse {
                time_saved: meta.duration,
                source: CacheSource::Local,
            },
            restored_files,
        ))
    }

    fn exists(&self, hash: &str) -> Result<CacheResponse, CacheError> {
        let uncompressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar", hash));
        let compressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar.zst", hash));

        if !uncompressed_cache_path.exists() && !compressed_cache_path.exists() {
            return Err(CacheError::CacheMiss);
        }

        let duration = CacheMetadata::read(
            &self
                .cache_directory
                .join_component(&format!("{}-meta.json", hash)),
        )
        .map(|meta| meta.duration)
        .unwrap_or(0);

        Ok(CacheResponse {
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

        let metadata_path = self
            .cache_directory
            .join_component(&format!("{}-meta.json", hash));

        let meta = CacheMetadata {
            hash: hash.to_string(),
            duration,
        };

        let mut metadata_options = OpenOptions::new();
        metadata_options.create(true).write(true);

        let metadata_file = metadata_path.open_with_options(metadata_options)?;

        serde_json::to_writer(metadata_file, &meta)
            .map_err(|e| CacheError::InvalidMetadata(e, Backtrace::capture()))?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::assert_matches::assert_matches;

    use anyhow::Result;
    use futures::future::try_join_all;
    use tempfile::tempdir;

    use super::*;
    use crate::test_cases::{get_test_cases, TestCase};

    #[tokio::test]
    async fn test_fs_cache() -> Result<()> {
        try_join_all(get_test_cases().into_iter().map(round_trip_test)).await?;

        Ok(())
    }

    async fn round_trip_test(test_case: TestCase) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        test_case.initialize(repo_root_path)?;

        let cache = FSCache::new(None, &repo_root_path)?;

        let expected_miss = cache
            .exists(&test_case.hash)
            .expect_err("Expected cache miss");
        assert_matches!(expected_miss, CacheError::CacheMiss);

        cache.put(
            repo_root_path,
            &test_case.hash,
            test_case.duration,
            test_case.files.iter().map(|f| f.path.clone()).collect(),
        )?;

        let expected_hit = cache.exists(&test_case.hash)?;
        assert_eq!(
            expected_hit,
            CacheResponse {
                time_saved: test_case.duration,
                source: CacheSource::Local
            }
        );

        let (status, files) = cache.fetch(&repo_root_path, &test_case.hash)?;
        assert_eq!(
            status,
            CacheResponse {
                time_saved: test_case.duration,
                source: CacheSource::Local
            }
        );

        assert_eq!(files.len(), test_case.files.len());
        for (expected, actual) in test_case.files.iter().zip(files.iter()) {
            assert_eq!(&expected.path, actual);
            let actual_file = repo_root_path.resolve(actual);
            assert_eq!(expected.contents, actual_file.read_to_string()?);
        }

        Ok(())
    }
}
