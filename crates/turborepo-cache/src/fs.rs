use std::{backtrace::Backtrace, time::Duration};

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{analytics, analytics::AnalyticsEvent};

use crate::{
    CacheError, CacheHitMetadata, CacheSource, LazyScmState,
    cache_archive::{CacheReader, CacheWriter},
};

pub struct FSCache {
    cache_directory: AbsoluteSystemPathBuf,
    analytics_recorder: Option<AnalyticsSender>,
    scm_state: LazyScmState,
}

#[derive(Debug, Deserialize, Serialize)]
struct CacheMetadata {
    hash: String,
    duration: u64,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    dirty_hash: Option<String>,
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
        scm_state: LazyScmState,
    ) -> Result<Self, CacheError> {
        debug!(
            "FSCache::new called with cache_dir={}, repo_root={}",
            cache_dir, repo_root
        );
        let cache_directory = Self::resolve_cache_dir(repo_root, cache_dir);
        debug!("FSCache resolved cache_directory={}", cache_directory);
        cache_directory.create_dir_all()?;

        Ok(FSCache {
            cache_directory,
            analytics_recorder,
            scm_state,
        })
    }

    pub(crate) fn cache_directory(&self) -> &AbsoluteSystemPath {
        &self.cache_directory
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
        let cache_path = self
            .cache_directory
            .join_component(&format!("{hash}.tar.zst"));

        // Check if the archive exists before doing any work.
        if !cache_path.as_path().exists() {
            self.log_fetch(analytics::CacheEvent::Miss, hash, 0);
            return Ok(None);
        }

        let manifest_path = self
            .cache_directory
            .join_component(&format!("{hash}-manifest.json"));

        let previous_manifest = crate::cache_archive::RestoreManifest::read(&manifest_path);

        // Fast path: if a manifest exists and ALL files on disk still match,
        // skip opening/decompressing the tar entirely.
        if let Some(ref manifest) = previous_manifest
            && let Some(file_list) = manifest.validate_all(anchor)
        {
            let meta = CacheMetadata::read(
                &self
                    .cache_directory
                    .join_component(&format!("{hash}-meta.json")),
            )?;
            self.log_fetch(analytics::CacheEvent::Hit, hash, meta.duration);
            return Ok(Some((
                CacheHitMetadata {
                    time_saved: meta.duration,
                    source: CacheSource::Local,
                    sha: meta.sha,
                    dirty_hash: meta.dirty_hash,
                },
                file_list,
            )));
        }

        // Slow path: decompress and extract the archive. Pass any existing
        // manifest so that individual files still matching on disk can be
        // skipped, avoiding unnecessary writes and filesystem notifications.
        let mut cache_reader = match CacheReader::open(&cache_path) {
            Ok(reader) => reader,
            Err(CacheError::IO(ref e, _)) if e.kind() == std::io::ErrorKind::NotFound => {
                self.log_fetch(analytics::CacheEvent::Miss, hash, 0);
                return Ok(None);
            }
            Err(e) => return Err(e),
        };

        let (restored_files, new_manifest) =
            cache_reader.restore(anchor, previous_manifest.as_ref())?;

        let manifest_path_owned = manifest_path.to_owned();
        std::thread::spawn(move || {
            let _ = new_manifest.write_atomic(&manifest_path_owned);
        });

        let meta = CacheMetadata::read(
            &self
                .cache_directory
                .join_component(&format!("{hash}-meta.json")),
        )?;

        self.log_fetch(analytics::CacheEvent::Hit, hash, meta.duration);

        Ok(Some((
            CacheHitMetadata {
                time_saved: meta.duration,
                source: CacheSource::Local,
                sha: meta.sha,
                dirty_hash: meta.dirty_hash,
            },
            restored_files,
        )))
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn exists(&self, hash: &str) -> Result<Option<CacheHitMetadata>, CacheError> {
        let cache_dir = self.cache_directory.as_str();
        let mut buf = String::with_capacity(cache_dir.len() + 1 + hash.len() + "-meta.json".len());
        buf.push_str(cache_dir);
        buf.push(std::path::MAIN_SEPARATOR);
        buf.push_str(hash);
        let prefix_len = buf.len();

        buf.push_str(".tar.zst");
        if !std::path::Path::new(&buf).exists() {
            return Ok(None);
        }

        buf.truncate(prefix_len);
        buf.push_str("-meta.json");

        let meta = CacheMetadata::read(
            &AbsoluteSystemPathBuf::try_from(buf.as_str())
                .map_err(|_| CacheError::ConfigCacheInvalidBase)?,
        );

        let (duration, sha, dirty_hash) = match meta {
            Ok(m) => (m.duration, m.sha, m.dirty_hash),
            Err(_) => (0, None, None),
        };

        Ok(Some(CacheHitMetadata {
            time_saved: duration,
            source: CacheSource::Local,
            sha,
            dirty_hash,
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
            .join_component(&format!("{hash}.tar.zst"));

        let mut cache_item = CacheWriter::create(&cache_path)?;
        let mut manifest = crate::cache_archive::RestoreManifest::new();

        for file in files {
            cache_item.add_file(anchor, file)?;

            let source_path = anchor.resolve(file);
            let unix_path = file.to_unix();
            if let Ok(m) = source_path.symlink_metadata() {
                if m.is_file() {
                    let _ = manifest.record_file(unix_path.as_str().to_owned(), &source_path);
                } else if m.is_dir() {
                    manifest.record_dir(unix_path.as_str().to_owned());
                }
            }
        }

        // Finish the archive (performs atomic rename from temp to final path)
        cache_item.finish()?;

        // Write manifest alongside the archive so the first fetch() can
        // skip decompression when outputs are still on disk.
        let manifest_path = self
            .cache_directory
            .join_component(&format!("{hash}-manifest.json"));
        let _ = manifest.write_atomic(&manifest_path);

        // Write metadata file atomically using write-to-temp-then-rename pattern
        let metadata_path = self
            .cache_directory
            .join_component(&format!("{hash}-meta.json"));

        let resolved = self.scm_state.get();
        let meta = CacheMetadata {
            hash: hash.to_string(),
            duration,
            sha: resolved.and_then(|s| s.sha.clone()),
            dirty_hash: resolved.and_then(|s| s.dirty_hash.clone()),
        };

        let meta_json = serde_json::to_string(&meta)
            .map_err(|e| CacheError::InvalidMetadata(e, Backtrace::capture()))?;

        // Write to temporary file then atomically rename
        let temp_metadata_path = self
            .cache_directory
            .join_component(&format!(".{hash}-meta.json.{}.tmp", std::process::id()));

        temp_metadata_path.create_with_contents(&meta_json)?;
        temp_metadata_path.rename(&metadata_path)?;

        Ok(())
    }

    pub fn evict(&self, max_age: Option<Duration>, max_size: Option<u64>) -> (u64, u64) {
        evict_cache_dir(&self.cache_directory, max_age, max_size)
    }
}

/// Evicts cache entries from the given directory based on age and/or
/// total size constraints.
///
/// Phase 1 (TTL): If `max_age` is `Some`, removes entries whose
/// `.tar.zst` archive mtime is older than the cutoff. Orphaned `.tmp`
/// files from crashed writes are cleaned up if older than 1 hour.
///
/// Phase 2 (LRU): If `max_size` is `Some` and the remaining cache
/// exceeds the limit, deletes the oldest entries first until the
/// total size is under the cap. Size includes sidecar files
/// (`-meta.json`, `-manifest.json`).
///
/// Returns the number of entries removed and bytes reclaimed.
/// Eviction is best-effort: individual file removal failures are
/// silently skipped.
pub(crate) fn evict_cache_dir(
    cache_directory: &AbsoluteSystemPath,
    max_age: Option<Duration>,
    max_size: Option<u64>,
) -> (u64, u64) {
    let now = std::time::SystemTime::now();

    let entries = match std::fs::read_dir(cache_directory.as_std_path()) {
        Ok(entries) => entries,
        Err(_) => return (0, 0),
    };

    let mut removed_count: u64 = 0;
    let mut reclaimed_bytes: u64 = 0;

    struct ArchiveEntry {
        hash: String,
        size: u64,
        mtime: std::time::SystemTime,
    }
    let mut remaining: Vec<ArchiveEntry> = Vec::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.ends_with(".tmp") {
            // Only clean up orphaned .tmp files older than 1 hour to avoid
            // racing with in-progress writes from concurrent processes.
            if let Ok(meta) = entry.metadata() {
                let is_stale = meta
                    .modified()
                    .ok()
                    .and_then(|mtime| now.duration_since(mtime).ok())
                    .is_some_and(|age| age >= Duration::from_secs(3600));

                if is_stale {
                    reclaimed_bytes += meta.len();
                    let _ = std::fs::remove_file(entry.path());
                }
            }
            continue;
        }

        if !name_str.ends_with(".tar.zst") {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let mtime = match metadata.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let hash = name_str.trim_end_matches(".tar.zst").to_owned();
        let mut entry_size = metadata.len();

        // Include sidecar sizes for accurate budget tracking
        let meta_path = cache_directory.join_component(&format!("{hash}-meta.json"));
        if let Ok(m) = std::fs::symlink_metadata(meta_path.as_std_path()) {
            entry_size += m.len();
        }
        let manifest_path = cache_directory.join_component(&format!("{hash}-manifest.json"));
        if let Ok(m) = std::fs::symlink_metadata(manifest_path.as_std_path()) {
            entry_size += m.len();
        }

        // Phase 1: TTL eviction
        if let Some(max_age) = max_age {
            let cutoff = now.checked_sub(max_age).unwrap_or(std::time::UNIX_EPOCH);
            if mtime < cutoff {
                remove_cache_entry(cache_directory, &hash, entry_size, &mut reclaimed_bytes);
                removed_count += 1;
                continue;
            }
        }

        remaining.push(ArchiveEntry {
            hash,
            size: entry_size,
            mtime,
        });
    }

    // Phase 2: LRU eviction by size
    if let Some(max_size) = max_size {
        let mut total_size: u64 = remaining.iter().map(|e| e.size).sum();

        if total_size > max_size {
            remaining.sort_by(|a, b| a.mtime.cmp(&b.mtime));

            for entry in &remaining {
                if total_size <= max_size {
                    break;
                }
                remove_cache_entry(
                    cache_directory,
                    &entry.hash,
                    entry.size,
                    &mut reclaimed_bytes,
                );
                removed_count += 1;
                total_size = total_size.saturating_sub(entry.size);
            }
        }
    }

    if removed_count > 0 {
        info!(
            "cache eviction: removed {} entries, reclaimed {} bytes",
            removed_count, reclaimed_bytes
        );
    } else {
        debug!("cache eviction: no entries removed");
    }

    (removed_count, reclaimed_bytes)
}

fn remove_cache_entry(
    cache_directory: &AbsoluteSystemPath,
    hash: &str,
    entry_size: u64,
    reclaimed_bytes: &mut u64,
) {
    let archive_path = cache_directory.join_component(&format!("{hash}.tar.zst"));
    let _ = std::fs::remove_file(archive_path.as_std_path());

    let meta_path = cache_directory.join_component(&format!("{hash}-meta.json"));
    let _ = std::fs::remove_file(meta_path.as_std_path());

    let manifest_path = cache_directory.join_component(&format!("{hash}-manifest.json"));
    let _ = std::fs::remove_file(manifest_path.as_std_path());

    *reclaimed_bytes += entry_size;
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use anyhow::Result;
    use futures::future::try_join_all;
    use tempfile::tempdir;
    use turbopath::AnchoredSystemPath;
    use turborepo_analytics::start_analytics;
    use turborepo_api_client::{APIAuth, APIClient, SecretString};
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::{
        CacheScmState,
        test_cases::{TestCase, get_test_cases, validate_analytics},
    };

    #[tokio::test]
    async fn test_fs_cache() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(start_test_server(port, Some(ready_tx)));

        // Wait for server to be ready
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start"))??;

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
            format!("http://localhost:{port}"),
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            true,
        )?;
        let api_auth = APIAuth {
            team_id: Some("my-team".to_string()),
            token: SecretString::new("my-token".to_string()),
            team_slug: None,
        };
        let (analytics_sender, analytics_handle) =
            start_analytics(api_auth.clone(), api_client.clone());

        let cache = FSCache::new(
            Utf8Path::new(""),
            repo_root_path,
            Some(analytics_sender.clone()),
            LazyScmState::resolved(None),
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
                source: CacheSource::Local,
                sha: None,
                dirty_hash: None,
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

    /// Test that multiple concurrent writes to the same hash don't corrupt the
    /// cache. This tests the atomic write pattern
    /// (write-to-temp-then-rename).
    #[tokio::test]
    async fn test_concurrent_writes_same_hash() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        // Create test files
        let test_file = repo_root_path.join_component("test.txt");
        test_file.create_with_contents("test content")?;

        let files = vec![AnchoredSystemPathBuf::from_raw("test.txt")?];
        let hash = "concurrent_write_test";
        let duration = 100;

        // Create multiple caches pointing to the same directory
        let cache1 = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let cache2 = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let cache3 = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;

        // Perform concurrent writes
        let handle1 = {
            let files = files.clone();
            let repo_root = repo_root_path.to_owned();
            tokio::spawn(async move { cache1.put(&repo_root, hash, &files, duration) })
        };
        let handle2 = {
            let files = files.clone();
            let repo_root = repo_root_path.to_owned();
            tokio::spawn(async move { cache2.put(&repo_root, hash, &files, duration) })
        };
        let handle3 = {
            let files = files.clone();
            let repo_root = repo_root_path.to_owned();
            tokio::spawn(async move { cache3.put(&repo_root, hash, &files, duration) })
        };

        // All writes should succeed (or at least not corrupt the cache)
        let _ = handle1.await?;
        let _ = handle2.await?;
        let _ = handle3.await?;

        // The cache should be readable
        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let result = cache.fetch(repo_root_path, hash)?;
        assert!(
            result.is_some(),
            "Cache should be readable after concurrent writes"
        );

        Ok(())
    }

    /// Test that reads during writes don't fail.
    /// A read should either return the old content, new content, or a miss -
    /// never corrupted data.
    #[tokio::test]
    async fn test_read_during_write() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        // Create test files
        let test_file = repo_root_path.join_component("test.txt");
        test_file.create_with_contents("original content")?;

        let files = vec![AnchoredSystemPathBuf::from_raw("test.txt")?];
        let hash = "read_during_write_test";
        let duration = 100;

        // First write to establish the cache
        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        cache.put(repo_root_path, hash, &files, duration)?;

        // Update the source file
        test_file.create_with_contents("updated content")?;

        // Perform concurrent read and write
        let cache_write = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let cache_read = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;

        let write_handle = {
            let files = files.clone();
            let repo_root = repo_root_path.to_owned();
            tokio::spawn(async move { cache_write.put(&repo_root, hash, &files, duration + 1) })
        };

        // Perform multiple reads while write is happening
        for _ in 0..10 {
            let result = cache_read.fetch(repo_root_path, hash);
            // Should either succeed with valid data or fail cleanly - no corruption
            if let Ok(Some((metadata, _))) = result {
                // Duration should be either old or new value
                assert!(
                    metadata.time_saved == duration || metadata.time_saved == duration + 1,
                    "Unexpected duration: {}",
                    metadata.time_saved
                );
            }
        }

        write_handle.await??;

        Ok(())
    }

    /// Test that multiple concurrent reads don't interfere with each other.
    #[tokio::test]
    async fn test_concurrent_reads() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        // Create test files
        let test_file = repo_root_path.join_component("test.txt");
        test_file.create_with_contents("test content")?;

        let files = vec![AnchoredSystemPathBuf::from_raw("test.txt")?];
        let hash = "concurrent_read_test";
        let duration = 100;

        // Write to cache first
        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        cache.put(repo_root_path, hash, &files, duration)?;

        // Perform concurrent reads
        let mut handles = Vec::new();
        for _ in 0..10 {
            let cache = FSCache::new(
                Utf8Path::new("cache"),
                repo_root_path,
                None,
                LazyScmState::resolved(None),
            )?;
            let repo_root = repo_root_path.to_owned();
            handles.push(tokio::spawn(async move { cache.fetch(&repo_root, hash) }));
        }

        // All reads should succeed
        for handle in handles {
            let result = handle.await??;
            assert!(result.is_some(), "Concurrent read should succeed");
            let (metadata, _) = result.unwrap();
            assert_eq!(metadata.time_saved, duration);
        }

        Ok(())
    }

    /// Test that temp files are cleaned up after concurrent writes.
    #[tokio::test]
    async fn test_concurrent_writes_cleanup_temp_files() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        // Create test files
        let test_file = repo_root_path.join_component("test.txt");
        test_file.create_with_contents("test content")?;

        let files = vec![AnchoredSystemPathBuf::from_raw("test.txt")?];
        let hash = "temp_cleanup_test";
        let duration = 100;

        // Perform concurrent writes
        let cache1 = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let cache2 = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let cache3 = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;

        let handle1 = {
            let files = files.clone();
            let repo_root = repo_root_path.to_owned();
            tokio::spawn(async move { cache1.put(&repo_root, hash, &files, duration) })
        };
        let handle2 = {
            let files = files.clone();
            let repo_root = repo_root_path.to_owned();
            tokio::spawn(async move { cache2.put(&repo_root, hash, &files, duration) })
        };
        let handle3 = {
            let files = files.clone();
            let repo_root = repo_root_path.to_owned();
            tokio::spawn(async move { cache3.put(&repo_root, hash, &files, duration) })
        };

        // Wait for all writes to complete
        let _ = handle1.await?;
        let _ = handle2.await?;
        let _ = handle3.await?;

        // Verify no orphaned temp files remain in cache directory
        let cache_dir = repo_root_path.join_component("cache");
        let temp_files: Vec<_> = std::fs::read_dir(cache_dir.as_std_path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            temp_files.is_empty(),
            "Orphaned temp files found after concurrent writes: {:?}",
            temp_files
        );

        // Verify exactly one archive file exists for the hash
        let archive_files: Vec<_> = std::fs::read_dir(cache_dir.as_std_path())?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.contains(hash) && name.ends_with(".tar.zst")
            })
            .collect();
        assert_eq!(
            archive_files.len(),
            1,
            "Expected exactly one archive file, found: {:?}",
            archive_files
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_fs_cache_writes_scm_metadata() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        let test_file = repo_root_path.join_component("test.txt");
        test_file.create_with_contents("content")?;

        let scm_state = LazyScmState::resolved(Some(CacheScmState {
            sha: Some("abc123def456".to_string()),
            dirty_hash: Some("fedcba654321".to_string()),
        }));
        let cache = FSCache::new(Utf8Path::new("cache"), repo_root_path, None, scm_state)?;
        let files = vec![AnchoredSystemPathBuf::from_raw("test.txt")?];
        cache.put(repo_root_path, "scm-test-hash", &files, 42)?;

        let meta_path = repo_root_path
            .join_component("cache")
            .join_component("scm-test-hash-meta.json");
        let meta_json: serde_json::Value = serde_json::from_str(&meta_path.read_to_string()?)?;
        assert_eq!(meta_json["sha"], "abc123def456");
        assert_eq!(meta_json["dirty_hash"], "fedcba654321");
        assert_eq!(meta_json["duration"], 42);
        assert_eq!(meta_json["hash"], "scm-test-hash");

        Ok(())
    }

    #[tokio::test]
    async fn test_fs_cache_writes_null_scm_fields_when_none() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        let test_file = repo_root_path.join_component("test.txt");
        test_file.create_with_contents("content")?;

        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let files = vec![AnchoredSystemPathBuf::from_raw("test.txt")?];
        cache.put(repo_root_path, "no-scm-hash", &files, 10)?;

        let meta_path = repo_root_path
            .join_component("cache")
            .join_component("no-scm-hash-meta.json");
        let meta_json: serde_json::Value = serde_json::from_str(&meta_path.read_to_string()?)?;
        assert_eq!(meta_json["sha"], serde_json::Value::Null);
        assert_eq!(meta_json["dirty_hash"], serde_json::Value::Null);

        Ok(())
    }

    #[test]
    fn test_cache_metadata_deserializes_without_scm_fields() {
        let old_json = r#"{"hash":"abc123","duration":100}"#;
        let meta: CacheMetadata = serde_json::from_str(old_json).unwrap();
        assert_eq!(meta.hash, "abc123");
        assert_eq!(meta.duration, 100);
        assert!(meta.sha.is_none());
        assert!(meta.dirty_hash.is_none());
    }

    #[test]
    fn test_cache_metadata_round_trips_with_scm_fields() {
        let meta = CacheMetadata {
            hash: "abc".to_string(),
            duration: 99,
            sha: Some("deadbeef".to_string()),
            dirty_hash: Some("cafebabe".to_string()),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: CacheMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.sha, Some("deadbeef".to_string()));
        assert_eq!(deserialized.dirty_hash, Some("cafebabe".to_string()));
        assert_eq!(deserialized.hash, "abc");
        assert_eq!(deserialized.duration, 99);
    }

    #[test]
    fn test_evict_removes_stale_entries() {
        let repo_root = tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path()).unwrap();
        let cache_dir = repo_root_path.join_component("cache");
        cache_dir.create_dir_all().unwrap();

        // Create "stale" entries: we backdate their mtime to 10 days ago
        let stale_hash = "stale_abc123";
        let stale_tar = cache_dir.join_component(&format!("{stale_hash}.tar.zst"));
        let stale_meta = cache_dir.join_component(&format!("{stale_hash}-meta.json"));
        let stale_manifest = cache_dir.join_component(&format!("{stale_hash}-manifest.json"));

        stale_tar
            .create_with_contents("stale archive data")
            .unwrap();
        stale_meta
            .create_with_contents(r#"{"hash":"stale_abc123","duration":100}"#)
            .unwrap();
        stale_manifest
            .create_with_contents(r#"{"files":{},"order":[]}"#)
            .unwrap();

        // Backdate mtime to 10 days ago
        let ten_days_ago = filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - Duration::from_secs(86400 * 10),
        );
        filetime::set_file_mtime(stale_tar.as_std_path(), ten_days_ago).unwrap();
        filetime::set_file_mtime(stale_meta.as_std_path(), ten_days_ago).unwrap();
        filetime::set_file_mtime(stale_manifest.as_std_path(), ten_days_ago).unwrap();

        // Create "fresh" entry
        let fresh_hash = "fresh_def456";
        let fresh_tar = cache_dir.join_component(&format!("{fresh_hash}.tar.zst"));
        let fresh_meta = cache_dir.join_component(&format!("{fresh_hash}-meta.json"));
        fresh_tar
            .create_with_contents("fresh archive data")
            .unwrap();
        fresh_meta
            .create_with_contents(r#"{"hash":"fresh_def456","duration":50}"#)
            .unwrap();

        // Create an orphaned temp file and backdate it past the 1-hour threshold
        let tmp_file = cache_dir.join_component(".some_hash.tar.zst.12345.0.tmp");
        tmp_file.create_with_contents("orphaned temp").unwrap();
        let two_hours_ago = filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - Duration::from_secs(7200),
        );
        filetime::set_file_mtime(tmp_file.as_std_path(), two_hours_ago).unwrap();

        let cache = FSCache::new(
            camino::Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        // Evict entries older than 7 days
        let (removed, reclaimed) = cache.evict(Some(Duration::from_secs(86400 * 7)), None);

        assert_eq!(removed, 1, "should remove exactly one stale entry");
        assert!(reclaimed > 0, "should reclaim some bytes");

        // Stale files should be gone
        assert!(!stale_tar.exists(), "stale archive should be removed");
        assert!(!stale_meta.exists(), "stale meta should be removed");
        assert!(!stale_manifest.exists(), "stale manifest should be removed");

        // Fresh files should remain
        assert!(fresh_tar.exists(), "fresh archive should remain");
        assert!(fresh_meta.exists(), "fresh meta should remain");

        // Temp file should be cleaned up
        assert!(!tmp_file.exists(), "orphaned temp file should be removed");
    }

    #[test]
    fn test_evict_keeps_everything_when_all_fresh() {
        let repo_root = tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path()).unwrap();
        let cache_dir = repo_root_path.join_component("cache");
        cache_dir.create_dir_all().unwrap();

        let hash = "all_fresh";
        let tar = cache_dir.join_component(&format!("{hash}.tar.zst"));
        let meta = cache_dir.join_component(&format!("{hash}-meta.json"));
        tar.create_with_contents("data").unwrap();
        meta.create_with_contents(r#"{"hash":"all_fresh","duration":10}"#)
            .unwrap();

        let cache = FSCache::new(
            camino::Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        let (removed, _) = cache.evict(Some(Duration::from_secs(86400 * 7)), None);
        assert_eq!(removed, 0);
        assert!(tar.exists());
        assert!(meta.exists());
    }

    #[test]
    fn test_evict_empty_cache() {
        let repo_root = tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path()).unwrap();

        let cache = FSCache::new(
            camino::Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        let (removed, reclaimed) = cache.evict(Some(Duration::from_secs(86400)), None);
        assert_eq!(removed, 0);
        assert_eq!(reclaimed, 0);
    }

    #[test]
    fn test_evict_by_size_removes_oldest_first() {
        let repo_root = tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path()).unwrap();
        let cache_dir = repo_root_path.join_component("cache");
        cache_dir.create_dir_all().unwrap();

        // Create 3 entries with known sizes and different mtimes.
        // Each archive is ~1000 bytes of content.
        let data = "x".repeat(1000);

        let old_hash = "oldest_entry";
        let old_tar = cache_dir.join_component(&format!("{old_hash}.tar.zst"));
        old_tar.create_with_contents(&data).unwrap();
        cache_dir
            .join_component(&format!("{old_hash}-meta.json"))
            .create_with_contents(r#"{"hash":"oldest_entry","duration":1}"#)
            .unwrap();

        let mid_hash = "middle_entry";
        let mid_tar = cache_dir.join_component(&format!("{mid_hash}.tar.zst"));
        mid_tar.create_with_contents(&data).unwrap();
        cache_dir
            .join_component(&format!("{mid_hash}-meta.json"))
            .create_with_contents(r#"{"hash":"middle_entry","duration":2}"#)
            .unwrap();

        let new_hash = "newest_entry";
        let new_tar = cache_dir.join_component(&format!("{new_hash}.tar.zst"));
        new_tar.create_with_contents(&data).unwrap();
        cache_dir
            .join_component(&format!("{new_hash}-meta.json"))
            .create_with_contents(r#"{"hash":"newest_entry","duration":3}"#)
            .unwrap();

        // Backdate mtimes to create a clear ordering
        let one_day = std::time::Duration::from_secs(86400);
        let now = std::time::SystemTime::now();
        filetime::set_file_mtime(
            old_tar.as_std_path(),
            filetime::FileTime::from_system_time(now - one_day * 3),
        )
        .unwrap();
        filetime::set_file_mtime(
            mid_tar.as_std_path(),
            filetime::FileTime::from_system_time(now - one_day * 2),
        )
        .unwrap();
        filetime::set_file_mtime(
            new_tar.as_std_path(),
            filetime::FileTime::from_system_time(now - one_day),
        )
        .unwrap();

        let cache = FSCache::new(
            camino::Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        // Set max size to ~1500 bytes — enough for 1 entry but not 2.
        // Total is ~3000 bytes, so we need to evict the 2 oldest.
        let (removed, _) = cache.evict(None, Some(1500));

        assert_eq!(removed, 2, "should evict the 2 oldest entries");
        assert!(!old_tar.exists(), "oldest should be removed");
        assert!(!mid_tar.exists(), "middle should be removed");
        assert!(new_tar.exists(), "newest should survive");
    }

    #[test]
    fn test_evict_size_noop_when_under_limit() {
        let repo_root = tempdir().unwrap();
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path()).unwrap();
        let cache_dir = repo_root_path.join_component("cache");
        cache_dir.create_dir_all().unwrap();

        let hash = "small_entry";
        let tar = cache_dir.join_component(&format!("{hash}.tar.zst"));
        tar.create_with_contents("tiny").unwrap();

        let cache = FSCache::new(
            camino::Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )
        .unwrap();

        // 10GB limit — way more than our tiny entry
        let (removed, _) = cache.evict(None, Some(10 * 1024 * 1024 * 1024));
        assert_eq!(removed, 0);
        assert!(tar.exists());
    }

    /// When the manifest fast path fails because one file changed, the slow
    /// path should still skip writing files that haven't changed. This
    /// prevents unnecessary filesystem notifications (inotify/fsevents) for
    /// unchanged outputs. See https://github.com/vercel/turborepo/issues/10875
    #[tokio::test]
    async fn test_slow_path_skips_unchanged_files() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        let stable_file = repo_root_path.join_component("stable.txt");
        stable_file.create_with_contents("unchanged content")?;

        let changing_file = repo_root_path.join_component("changing.txt");
        changing_file.create_with_contents("original")?;

        let files = vec![
            AnchoredSystemPathBuf::from_raw("stable.txt")?,
            AnchoredSystemPathBuf::from_raw("changing.txt")?,
        ];
        let hash = "partial_change_test";

        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        cache.put(repo_root_path, hash, &files, 50)?;

        let stable_mtime_before = stable_file.symlink_metadata()?.modified()?;

        // Ensure a measurable mtime gap so the overwrite (if any) is detectable.
        std::thread::sleep(Duration::from_millis(50));

        // Modify the changing file so the manifest fast-path fails.
        changing_file.create_with_contents("modified")?;

        // Fetch should hit the slow path (manifest validation fails because
        // changing.txt has a different mtime) but pass the manifest through
        // so stable.txt is skipped.
        let result = cache.fetch(repo_root_path, hash)?;
        assert!(result.is_some(), "Cache should still hit");

        // stable.txt should NOT have been rewritten — its mtime must be
        // identical to what it was before the fetch.
        let stable_mtime_after = stable_file.symlink_metadata()?.modified()?;
        assert_eq!(
            stable_mtime_before, stable_mtime_after,
            "Unchanged file should not be rewritten during slow-path restore"
        );

        // changing.txt should be restored to its original cached content.
        assert_eq!(changing_file.read_to_string()?, "original");

        Ok(())
    }
}
