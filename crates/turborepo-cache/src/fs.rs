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

/// Outcome of [`FSCache::lookup`].
pub(crate) enum LocalLookup {
    /// There is no archive for the hash.
    Miss,
    /// Every output recorded by the restore manifest is unchanged on disk, so
    /// the archive need not be opened.
    Unchanged(CacheHitMetadata, Vec<AnchoredSystemPathBuf>),
    /// The archive must be extracted with [`FSCache::restore`], passing the
    /// manifest so unchanged files can still be skipped.
    Restore(Option<crate::cache_archive::RestoreManifest>),
    /// The manifest is over the caller's size limit and was not read.
    ManifestTooLarge,
}

/// Uniquifies temporary archive names for concurrent installs of distinct
/// hashes within this process.
static ARCHIVE_TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
        match self.lookup(anchor, hash, u64::MAX)? {
            LocalLookup::Miss => Ok(None),
            LocalLookup::Unchanged(metadata, files) => Ok(Some((metadata, files))),
            LocalLookup::Restore(previous_manifest) => {
                self.restore(anchor, hash, previous_manifest)
            }
            // Unreachable with an unlimited manifest size, but restoring is
            // always correct.
            LocalLookup::ManifestTooLarge => self.restore(anchor, hash, None),
        }
    }

    /// First step of [`Self::fetch`]. It only reads metadata: the archive's
    /// existence, the restore manifest, one stat per recorded output, and the
    /// archive's metadata sidecar. It never opens the archive.
    ///
    /// Returns [`LocalLookup::ManifestTooLarge`] without reading a manifest
    /// larger than `max_manifest_bytes`, so callers can bound how much
    /// checking they do in place.
    #[tracing::instrument(skip_all)]
    pub(crate) fn lookup(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
        max_manifest_bytes: u64,
    ) -> Result<LocalLookup, CacheError> {
        let cache_path = self
            .cache_directory
            .join_component(&format!("{hash}.tar.zst"));

        // Check if the archive exists before doing any work.
        if !cache_path.as_path().exists() {
            self.log_fetch(analytics::CacheEvent::Miss, hash, 0);
            return Ok(LocalLookup::Miss);
        }

        let manifest_path = self
            .cache_directory
            .join_component(&format!("{hash}-manifest.json"));

        let Ok(previous_manifest) =
            crate::cache_archive::RestoreManifest::read_at_most(&manifest_path, max_manifest_bytes)
        else {
            return Ok(LocalLookup::ManifestTooLarge);
        };

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
            return Ok(LocalLookup::Unchanged(
                CacheHitMetadata {
                    time_saved: meta.duration,
                    source: CacheSource::Local,
                    sha: meta.sha,
                    dirty_hash: meta.dirty_hash,
                },
                file_list,
            ));
        }

        Ok(LocalLookup::Restore(previous_manifest))
    }

    /// Second step of [`Self::fetch`]: decompress and extract the archive.
    /// `previous_manifest` lets individual files that still match on disk be
    /// skipped, avoiding unnecessary writes and filesystem notifications.
    #[tracing::instrument(skip_all)]
    pub(crate) fn restore(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
        previous_manifest: Option<crate::cache_archive::RestoreManifest>,
    ) -> Result<Option<(CacheHitMetadata, Vec<AnchoredSystemPathBuf>)>, CacheError> {
        let cache_path = self
            .cache_directory
            .join_component(&format!("{hash}.tar.zst"));
        let manifest_path = self
            .cache_directory
            .join_component(&format!("{hash}-manifest.json"));

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

    /// Writes the `-manifest.json` and `-meta.json` sidecars for an installed
    /// archive.
    fn write_sidecars(
        &self,
        hash: &str,
        manifest: &crate::cache_archive::RestoreManifest,
        duration: u64,
    ) -> Result<(), CacheError> {
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
            // Record the manifest from what was archived instead of statting
            // each output again.
            let entry = cache_item.add_file(anchor, file)?;
            let unix_path = file.to_unix();
            match entry {
                crate::cache_archive::ArchivedEntry::File(m) => {
                    let _ = manifest.record_file_metadata(unix_path.as_str().to_owned(), &m);
                }
                crate::cache_archive::ArchivedEntry::Directory => {
                    manifest.record_dir(unix_path.as_str().to_owned());
                }
                crate::cache_archive::ArchivedEntry::Symlink(target) => {
                    manifest.record_symlink(unix_path.as_str().to_owned(), target.as_str());
                }
                crate::cache_archive::ArchivedEntry::Other => {}
            }
        }

        // Finish the archive (performs atomic rename from temp to final path)
        cache_item.finish()?;

        self.write_sidecars(hash, &manifest, duration)?;

        Ok(())
    }

    /// Atomically installs an already-built compressed archive, avoiding a
    /// second read/compress pass when the same artifact has another
    /// destination (a remote write or a verified remote download).
    ///
    /// `files` must already exist under `anchor` so the restore manifest can
    /// be recorded from their current metadata.
    #[tracing::instrument(skip_all)]
    pub(crate) fn put_archive(
        &self,
        anchor: &AbsoluteSystemPath,
        hash: &str,
        files: &[AnchoredSystemPathBuf],
        archive: &crate::artifact_body::ArtifactBody,
        duration: u64,
    ) -> Result<(), CacheError> {
        let cache_path = self
            .cache_directory
            .join_component(&format!("{hash}.tar.zst"));

        // Write to a temporary sibling and atomically rename, matching the
        // publication semantics of CacheWriter::create.
        let temp_path = self.cache_directory.join_component(&format!(
            ".{hash}.tar.zst.{}.{}.tmp",
            std::process::id(),
            ARCHIVE_TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        {
            let file = temp_path.create()?;
            // Match the 1 MiB flush granularity of CacheWriter::create so the
            // copy does not turn into per-8-KiB write syscalls.
            let mut buffered = std::io::BufWriter::with_capacity(2usize.pow(20), file);
            archive.copy_to(&mut buffered)?;
            // Flush explicitly; BufWriter's Drop would ignore the error.
            std::io::Write::flush(&mut buffered)?;
        }
        temp_path.rename(&cache_path)?;

        let manifest = crate::cache_archive::RestoreManifest::from_disk(anchor, files);
        self.write_sidecars(hash, &manifest, duration)?;

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
            remaining.sort_by_key(|a| a.mtime);

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
    use turborepo_api_client::{APIAuth, APIClient};
    use turborepo_types::SecretString;
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::{
        CacheScmState,
        test_cases::{TestCase, get_test_cases, validate_analytics},
    };

    #[tokio::test]
    async fn test_fs_cache() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
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

    #[test]
    #[cfg(unix)]
    fn put_allows_file_through_internal_symlinked_parent() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        let app_dir = repo_root_path
            .join_component("packages")
            .join_component("app");
        app_dir.create_dir_all()?;

        let actual_dist = app_dir.join_component("actual-dist");
        actual_dist.create_dir_all()?;
        actual_dist
            .join_component("file.txt")
            .create_with_contents("hello")?;
        app_dir
            .join_component("dist")
            .symlink_to_dir("actual-dist")?;
        let log_dir = app_dir.join_component(".turbo");
        log_dir.create_dir_all()?;
        log_dir
            .join_component("turbo-build.log")
            .create_with_contents("log")?;

        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        let files = vec![
            AnchoredSystemPathBuf::from_raw("packages/app/.turbo/turbo-build.log")?,
            AnchoredSystemPathBuf::from_raw("packages/app/dist")?,
            AnchoredSystemPathBuf::from_raw("packages/app/dist/file.txt")?,
        ];

        cache.put(repo_root_path, "symlinked-parent", &files, 50)?;
        let result = cache.fetch(repo_root_path, "symlinked-parent")?;

        assert!(result.is_some());

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

    /// Lays out `dist/` with files, an empty directory, and symlinks, and
    /// returns the paths a task caching `dist/**` would save. Restoring them
    /// into an empty `dist/` writes `link` as soon as it is read, and defers
    /// `dangling` and `early` (whose target comes later in the archive) until
    /// every other entry is restored.
    #[cfg(unix)]
    fn symlinked_outputs(repo_root: &AbsoluteSystemPath) -> Result<Vec<AnchoredSystemPathBuf>> {
        let dist = repo_root.join_component("dist");
        dist.join_component("empty").create_dir_all()?;
        dist.join_component("a.txt").create_with_contents("a")?;
        dist.join_component("b.txt").create_with_contents("b")?;
        dist.join_component("late.txt")
            .create_with_contents("late")?;
        dist.join_component("link").symlink_to_file("a.txt")?;
        dist.join_component("dangling").symlink_to_file("missing")?;
        dist.join_component("early").symlink_to_file("late.txt")?;
        [
            "dist",
            "dist/a.txt",
            "dist/b.txt",
            "dist/dangling",
            "dist/early",
            "dist/empty",
            "dist/late.txt",
            "dist/link",
        ]
        .into_iter()
        .map(|path| Ok(AnchoredSystemPathBuf::from_raw(path)?))
        .collect()
    }

    /// A cache hit must leave every cached symlink in place: the manifest
    /// fast path may only skip the restore when symlinks match too. Both ways
    /// an archive is installed locally (saving outputs, and installing a
    /// remote hit) write the manifest the fast path trusts.
    #[test]
    #[cfg(unix)]
    fn fetch_restores_symlinks_changed_since_put() -> Result<()> {
        for (installer, change) in ["put", "put_archive"].into_iter().flat_map(|installer| {
            ["delete", "delete dangling", "retarget"].map(|change| (installer, change))
        }) {
            let repo_root = tempdir()?;
            let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;
            let files = symlinked_outputs(repo_root_path)?;
            let cache = FSCache::new(
                Utf8Path::new("cache"),
                repo_root_path,
                None,
                LazyScmState::resolved(None),
            )?;
            if installer == "put" {
                cache.put(repo_root_path, "symlinks", &files, 50)?;
            } else {
                let body = crate::artifact_body::ArtifactBody::from_files(repo_root_path, &files)?;
                cache.put_archive(repo_root_path, "symlinks", &files, &body, 50)?;
            }

            let dist = repo_root_path.join_component("dist");
            match change {
                "delete" => dist.join_component("link").remove_file()?,
                "delete dangling" => dist.join_component("dangling").remove_file()?,
                _ => {
                    dist.join_component("link").remove_file()?;
                    dist.join_component("link").symlink_to_file("b.txt")?;
                }
            }

            let (_, restored) = cache
                .fetch(repo_root_path, "symlinks")?
                .expect("cache should hit");
            assert_eq!(
                dist.join_component("link").read_link()?.as_str(),
                "a.txt",
                "{installer}, {change}: symlink should be restored"
            );
            assert_eq!(
                dist.join_component("dangling").read_link()?.as_str(),
                "missing",
                "{installer}, {change}: dangling symlink should be restored"
            );
            assert!(restored.contains(&AnchoredSystemPathBuf::from_raw("dist/link")?));
        }
        Ok(())
    }

    /// The manifest written after a restore records the same entries as the
    /// one written at save time, including directories and symlinks.
    #[test]
    #[cfg(unix)]
    fn restore_manifest_records_directories_and_symlinks() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        let files = symlinked_outputs(repo_root_path)?;
        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        cache.put(repo_root_path, "restored", &files, 50)?;

        let dist = repo_root_path.join_component("dist");
        dist.remove_dir_all()?;
        let archive = cache.cache_directory().join_component("restored.tar.zst");
        let (restored, manifest) = CacheReader::open(&archive)?.restore(repo_root_path, None)?;
        assert_eq!(
            manifest.validate_all(repo_root_path),
            Some(restored),
            "the fast path should report what the restore did"
        );

        dist.join_component("empty").remove_dir()?;
        assert!(
            manifest.validate_all(repo_root_path).is_none(),
            "a deleted directory must invalidate the manifest"
        );
        dist.join_component("empty").create_dir_all()?;
        assert!(manifest.validate_all(repo_root_path).is_some());

        // `link` is restored as soon as it is read; `dangling` and `early`
        // only once every other entry is restored.
        for link in ["link", "dangling", "early"] {
            let link = dist.join_component(link);
            let target = link.read_link()?;
            link.remove_file()?;
            assert!(
                manifest.validate_all(repo_root_path).is_none(),
                "deleting {link} must invalidate the manifest"
            );
            link.symlink_to_file("b.txt")?;
            assert!(
                manifest.validate_all(repo_root_path).is_none(),
                "retargeting {link} must invalidate the manifest"
            );
            link.remove_file()?;
            link.symlink_to_file(target.as_str())?;
            assert!(manifest.validate_all(repo_root_path).is_some());
        }

        // A restore that skips unchanged files still records them.
        dist.join_component("link").remove_file()?;
        let (restored, manifest) =
            CacheReader::open(&archive)?.restore(repo_root_path, Some(&manifest))?;
        assert_eq!(manifest.validate_all(repo_root_path), Some(restored));
        Ok(())
    }

    /// Manifests written before symlinks were recorded cannot vouch for
    /// them, so they must not short-circuit a restore, but their regular
    /// files still spare unchanged outputs from being rewritten.
    #[test]
    #[cfg(unix)]
    fn fetch_ignores_manifest_without_version() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        let files = symlinked_outputs(repo_root_path)?;
        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        cache.put(repo_root_path, "legacy", &files, 50)?;

        // Rewrite the manifest the way an older turbo wrote it: no version
        // and no symlink entries.
        let manifest_path = cache
            .cache_directory()
            .join_component("legacy-manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&manifest_path.read_to_string()?)?;
        let manifest = manifest.as_object_mut().unwrap();
        manifest.remove("version");
        let links = manifest.remove("link_targets").unwrap_or_default();
        let links = links.as_object().cloned().unwrap_or_default();
        let entries = manifest["files"].as_object_mut().unwrap();
        entries.retain(|path, _| !links.contains_key(path));
        let kept = entries
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        manifest["order"]
            .as_array_mut()
            .unwrap()
            .retain(|path| kept.contains(path.as_str().unwrap()));
        manifest_path.create_with_contents(serde_json::to_string(&manifest)?)?;

        let dist = repo_root_path.join_component("dist");
        let unchanged = dist.join_component("a.txt");
        let unchanged_mtime = unchanged.symlink_metadata()?.modified()?;
        std::thread::sleep(Duration::from_millis(50));
        let link = dist.join_component("link");
        link.remove_file()?;
        cache
            .fetch(repo_root_path, "legacy")?
            .expect("cache should hit");
        assert_eq!(link.read_link()?.as_str(), "a.txt");
        assert_eq!(
            unchanged.symlink_metadata()?.modified()?,
            unchanged_mtime,
            "unchanged file should not be rewritten"
        );
        Ok(())
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

    #[test]
    fn lookup_reports_each_outcome() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        let output = repo_root_path.join_component("output.txt");
        output.create_with_contents("cached")?;
        let files = vec![AnchoredSystemPathBuf::from_raw("output.txt")?];
        let hash = "lookup_outcomes";

        let cache = FSCache::new(
            Utf8Path::new("cache"),
            repo_root_path,
            None,
            LazyScmState::resolved(None),
        )?;
        assert!(matches!(
            cache.lookup(repo_root_path, hash, u64::MAX)?,
            LocalLookup::Miss
        ));

        cache.put(repo_root_path, hash, &files, 50)?;
        match cache.lookup(repo_root_path, hash, u64::MAX)? {
            LocalLookup::Unchanged(metadata, restored) => {
                assert_eq!(metadata.time_saved, 50);
                assert_eq!(restored, files);
            }
            _ => panic!("unchanged outputs should not need a restore"),
        }
        assert!(matches!(
            cache.lookup(repo_root_path, hash, 1)?,
            LocalLookup::ManifestTooLarge
        ));

        // A changed output (different size) needs a restore, which keeps the
        // manifest so unchanged files can still be skipped.
        output.create_with_contents("changed")?;
        let LocalLookup::Restore(Some(previous_manifest)) =
            cache.lookup(repo_root_path, hash, u64::MAX)?
        else {
            panic!("a changed output should need a restore with the manifest");
        };
        let (metadata, restored) = cache
            .restore(repo_root_path, hash, Some(previous_manifest))?
            .expect("the archive exists");
        assert_eq!(metadata.time_saved, 50);
        assert_eq!(restored, files);
        assert_eq!(output.read_to_string()?, "cached");

        Ok(())
    }
}
