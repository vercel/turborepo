use std::{backtrace::Backtrace, fs::OpenOptions};

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{analytics, analytics::AnalyticsEvent};

use crate::{
    cache_archive::{CacheReader, CacheWriter},
    CacheError, CacheHitMetadata, CacheSource,
};

pub struct FSCache {
    cache_directory: AbsoluteSystemPathBuf,
    analytics_recorder: Option<AnalyticsSender>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CacheMetadata {
    hash: String,
    duration: u64,
}

impl CacheMetadata {
    fn read(path: &AbsoluteSystemPath) -> Result<CacheMetadata, CacheError> {
        serde_json::from_str(&path.read_to_string()?)
            .map_err(|e| CacheError::InvalidMetadata(e, Backtrace::capture()))
    }
}

impl FSCache {
    fn resolve_cache_dir(
        repo_root: &AbsoluteSystemPath,
        cache_dir: &Utf8Path,
    ) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::from_unknown(repo_root, cache_dir)
    }

    #[tracing::instrument(skip_all)]
    pub fn new(
        cache_dir: &Utf8Path,
        repo_root: &AbsoluteSystemPath,
        analytics_recorder: Option<AnalyticsSender>,
    ) -> Result<Self, CacheError> {
        let cache_directory = Self::resolve_cache_dir(repo_root, cache_dir);
        cache_directory.create_dir_all()?;

        Ok(FSCache {
            cache_directory,
            analytics_recorder,
        })
    }

    fn log_fetch(&self, event: analytics::CacheEvent, hash: &str, duration: u64) {
        // If analytics fails to record, it's not worth failing the cache
        if let Some(analytics_recorder) = &self.analytics_recorder {
            let analytics_event = AnalyticsEvent {
                session_id: None,
                source: analytics::CacheSource::Local,
                event,
                hash: hash.to_string(),
                duration,
            };

            let _ = analytics_recorder.send(analytics_event);
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn fetch(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
    ) -> Result<Option<(CacheHitMetadata, Vec<AnchoredSystemPathBuf>)>, CacheError> {
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
            self.log_fetch(analytics::CacheEvent::Miss, hash, 0);
            return Ok(None);
        };

        let mut cache_reader = CacheReader::open(&cache_path)?;

        let restored_files = cache_reader.restore(anchor)?;

        let meta = CacheMetadata::read(
            &self
                .cache_directory
                .join_component(&format!("{}-meta.json", hash)),
        )?;

        self.log_fetch(analytics::CacheEvent::Hit, hash, meta.duration);

        Ok(Some((
            CacheHitMetadata {
                time_saved: meta.duration,
                source: CacheSource::Local,
            },
            restored_files,
        )))
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn exists(&self, hash: &str) -> Result<Option<CacheHitMetadata>, CacheError> {
        let uncompressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar", hash));
        let compressed_cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar.zst", hash));

        if !uncompressed_cache_path.exists() && !compressed_cache_path.exists() {
            return Ok(None);
        }

        let duration = CacheMetadata::read(
            &self
                .cache_directory
                .join_component(&format!("{}-meta.json", hash)),
        )
        .map(|meta| meta.duration)
        .unwrap_or(0);

        Ok(Some(CacheHitMetadata {
            time_saved: duration,
            source: CacheSource::Local,
        }))
    }

    #[tracing::instrument(skip_all)]
    pub fn put(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
        files: &[AnchoredSystemPathBuf],
        duration: u64,
    ) -> Result<(), CacheError> {
        let cache_path = self
            .cache_directory
            .join_component(&format!("{}.tar.zst", hash));

        let mut cache_item = CacheWriter::create(&cache_path)?;

        for file in files {
            cache_item.add_file(anchor, file)?;
        }

        let metadata_path = self
            .cache_directory
            .join_component(&format!("{}-meta.json", hash));

        let meta = CacheMetadata {
            hash: hash.to_string(),
            duration,
        };

        let mut metadata_options = OpenOptions::new();
        metadata_options.create(true).write(true).truncate(true);

        let metadata_file = metadata_path.open_with_options(metadata_options)?;

        serde_json::to_writer(metadata_file, &meta)
            .map_err(|e| CacheError::InvalidMetadata(e, Backtrace::capture()))?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use anyhow::Result;
    use futures::future::try_join_all;
    use tempfile::tempdir;
    use turbopath::AnchoredSystemPath;
    use turborepo_analytics::start_analytics;
    use turborepo_api_client::{APIAuth, APIClient};
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::test_cases::{get_test_cases, validate_analytics, TestCase};

    #[tokio::test]
    async fn test_fs_cache() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        tokio::spawn(start_test_server(port));

        let test_cases = get_test_cases();

        try_join_all(
            test_cases
                .iter()
                .map(|test_case| round_trip_test(test_case, port)),
        )
        .await?;

        validate_analytics(&test_cases, analytics::CacheSource::Local, port).await?;
        Ok(())
    }

    async fn round_trip_test(test_case: &TestCase, port: u16) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        test_case.initialize(repo_root_path)?;

        let api_client = APIClient::new(
            format!("http://localhost:{}", port),
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            true,
        )?;
        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: "my-token".to_string(),
            team_slug: None,
        };
        let (analytics_sender, analytics_handle) =
            start_analytics(api_auth.clone(), api_client.clone());

        let cache = FSCache::new(
            Utf8Path::new(""),
            repo_root_path,
            Some(analytics_sender.clone()),
        )?;

        let expected_miss = cache.fetch(repo_root_path, test_case.hash)?;
        assert!(expected_miss.is_none());

        let files: Vec<_> = test_case
            .files
            .iter()
            .map(|f| f.path().to_owned())
            .collect();
        cache.put(repo_root_path, test_case.hash, &files, test_case.duration)?;

        let (status, files) = cache.fetch(repo_root_path, test_case.hash)?.unwrap();

        assert_eq!(
            status,
            CacheHitMetadata {
                time_saved: test_case.duration,
                source: CacheSource::Local
            }
        );

        assert_eq!(files.len(), test_case.files.len());
        for (expected, actual) in test_case.files.iter().zip(files.iter()) {
            let actual: &AnchoredSystemPath = actual;
            assert_eq!(expected.path(), actual);
            let actual_file = repo_root_path.resolve(actual);
            if let Some(contents) = expected.contents() {
                assert_eq!(contents, actual_file.read_to_string()?);
            } else {
                assert!(actual_file.exists());
            }
        }

        analytics_handle.close_with_timeout().await;
        Ok(())
    }
}
