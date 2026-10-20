use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use futures::{StreamExt, stream};
use tracing::{debug, info, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{APIAuth, APIClient};

use crate::{
    CacheConfig, CacheError, CacheHitMetadata, CacheOpts, LazyScmState,
    fs::{FSCache, LocalLookup},
    http::{HTTPCache, UploadMap},
};

/// Restore manifests up to this size (roughly 1,700 recorded outputs with
/// typical path lengths) are checked against disk in place by
/// [`CacheMultiplexer::fetch`]; larger ones on the blocking pool.
const MAX_IN_PLACE_MANIFEST_BYTES: u64 = 256 * 1024;

pub struct CacheMultiplexer {
    // We use an `AtomicBool` instead of removing the cache because that would require
    // wrapping the cache in a `Mutex` which would cause a lot of contention.
    // This does create a mild race condition where we might use the cache
    // even though another thread might be removing it, but that's fine.
    should_use_http_cache: AtomicBool,
    cache_config: CacheConfig,
    fs: Option<Arc<FSCache>>,
    http: Option<Arc<HTTPCache>>,
    scm_state: LazyScmState,
}

impl CacheMultiplexer {
    #[tracing::instrument(skip_all)]
    pub fn new(
        opts: &CacheOpts,
        repo_root: &AbsoluteSystemPath,
        api_client: Option<APIClient>,
        api_auth: Option<APIAuth>,
        analytics_recorder: Option<AnalyticsSender>,
        scm_state: LazyScmState,
    ) -> Result<Self, CacheError> {
        let use_fs_cache = opts.cache.local.should_use();
        let use_http_cache = opts.cache.remote.should_use();

        // Since the above two flags are not mutually exclusive it is possible to
        // configure yourself out of having a cache. We should tell you about it
        // but we shouldn't fail your build for that reason.
        if !use_fs_cache && !use_http_cache {
            turborepo_log::warn(
                turborepo_log::Source::turbo(turborepo_log::Subsystem::Cache),
                "no caches are enabled",
            )
            .emit();
        }

        debug!(
            "CacheMultiplexer::new creating FSCache with cache_dir={}, repo_root={}",
            opts.cache_dir, repo_root
        );
        let fs_cache = use_fs_cache
            .then(|| {
                FSCache::new(
                    &opts.cache_dir,
                    repo_root,
                    analytics_recorder.clone(),
                    scm_state.clone(),
                )
                .map(Arc::new)
            })
            .transpose()?;

        if (opts.cache_max_age.is_some() || opts.cache_max_size.is_some())
            && let Some(fs) = &fs_cache
        {
            let cache_dir = fs.cache_directory().to_owned();
            let max_age = opts.cache_max_age;
            let max_size = opts.cache_max_size;
            info!(
                ?max_age,
                ?max_size,
                "cache eviction enabled, running in background"
            );
            std::thread::spawn(move || {
                crate::fs::evict_cache_dir(&cache_dir, max_age, max_size);
            });
        }

        let http_cache = if use_http_cache {
            match (api_client, api_auth) {
                (Some(api_client), Some(api_auth)) => Some(Arc::new(HTTPCache::new(
                    api_client,
                    opts,
                    repo_root.to_owned(),
                    api_auth,
                    analytics_recorder.clone(),
                    scm_state.clone(),
                )?)),
                _ => None,
            }
        } else {
            None
        };

        Ok(CacheMultiplexer {
            should_use_http_cache: AtomicBool::new(http_cache.is_some()),
            cache_config: opts.cache,
            fs: fs_cache,
            http: http_cache,
            scm_state,
        })
    }

    // This is technically a TOCTOU bug, but at worst it'll cause
    // a few extra cache requests.
    fn get_http_cache(&self) -> Option<&Arc<HTTPCache>> {
        if self.should_use_http_cache.load(Ordering::Relaxed) {
            self.http.as_ref().filter(|http| !http.is_disabled())
        } else {
            None
        }
    }

    /// Treats the remote cache as read-only for the rest of the run, as if
    /// `remote:r` had been configured. Reads are unaffected, and a run that
    /// already skips remote writes is left unchanged.
    pub fn disable_remote_writes(&self) {
        if self.cache_config.remote.write
            && let Some(http) = &self.http
        {
            http.disable_writes_read_only();
        }
    }

    pub fn requests(&self) -> Option<Arc<Mutex<UploadMap>>> {
        self.http.as_ref().map(|http| http.requests())
    }

    #[tracing::instrument(skip_all)]
    pub async fn put(
        &self,
        anchor: &AbsoluteSystemPath,
        key: &str,
        files: &[AnchoredSystemPathBuf],
        duration: u64,
    ) -> Result<(), CacheError> {
        // Wait for the background SCM computation to finish so that both
        // the FS sidecar metadata and the HTTP headers carry provenance
        // info. This is a no-op when the state is already resolved.
        self.scm_state.get_resolved().await;

        // When both destinations are written, build and compress the archive
        // exactly once, then install it locally and upload the same bytes.
        if self.cache_config.local.write
            && self.cache_config.remote.write
            && let Some(fs) = &self.fs
            && let Some(http) = self.get_http_cache()
            && !http.uploads_disabled()
        {
            // Archive construction reads and compresses every output file;
            // keep that synchronous work off the Tokio runtime workers.
            let artifact = {
                let anchor = anchor.to_owned();
                let files = files.to_vec();
                tokio::task::spawn_blocking(move || {
                    crate::artifact_body::Artifact::from_files(&anchor, &files)
                })
                .await??
            };
            let upload = artifact.body().clone();

            // Install the archive locally and upload it at the same time.
            // Both read the body independently, and neither depends on the
            // other.
            let install = {
                let fs = fs.clone();
                let key = key.to_owned();
                tokio::task::spawn_blocking(move || fs.put_archive(&key, &artifact, duration))
            };
            let (install_result, http_result) =
                tokio::join!(install, http.put_body(key, upload, duration));
            install_result??;

            return match http_result {
                Err(CacheError::ApiClientError(
                    box turborepo_api_client::Error::CacheDisabled { .. },
                    ..,
                )) => {
                    warn!("failed to put to http cache: cache disabled");
                    self.should_use_http_cache.store(false, Ordering::Relaxed);
                    Ok(())
                }
                Err(CacheError::ForbiddenRemoteCacheWrite) if http.uploads_disabled() => Ok(()),
                Err(CacheError::RemoteCacheUnavailable) => Ok(()), // local write succeeded
                Err(e) => Err(e),
                Ok(()) => Ok(()),
            };
        }

        if self.cache_config.local.write
            && let Some(fs) = &self.fs
        {
            // Synchronous archive I/O + compression belongs on the blocking
            // pool; the AsyncCache semaphore bounds concurrency.
            let fs = fs.clone();
            let anchor = anchor.to_owned();
            let key = key.to_owned();
            let files = files.to_vec();
            tokio::task::spawn_blocking(move || fs.put(&anchor, &key, &files, duration)).await??;
        }

        let http_result = match self.get_http_cache() {
            // Uploads were disabled for this run, either because the token is
            // read-only or because an upload stayed forbidden after recovery.
            Some(http) if self.cache_config.remote.write && http.uploads_disabled() => None,
            Some(http) => {
                if self.cache_config.remote.write {
                    let http_result = http.put(anchor, key, files, duration).await;

                    Some(http_result)
                } else {
                    // Remote writes are disabled by configuration. The run
                    // header already reports the cache as read-only.
                    debug!("remote cache is read-only, skipping upload of {key}");
                    None
                }
            }
            _ => None,
        };

        match http_result {
            Some(Err(CacheError::ApiClientError(
                box turborepo_api_client::Error::CacheDisabled { .. },
                ..,
            ))) => {
                warn!("failed to put to http cache: cache disabled");
                self.should_use_http_cache.store(false, Ordering::Relaxed);
                Ok(())
            }
            Some(Err(CacheError::ForbiddenRemoteCacheWrite))
                if self
                    .http
                    .as_ref()
                    .is_some_and(|http| http.uploads_disabled()) =>
            {
                Ok(())
            }
            Some(Err(CacheError::RemoteCacheUnavailable)) if self.cache_config.local.write => {
                Ok(())
            }
            Some(Err(e)) => Err(e),
            None | Some(Ok(())) => Ok(()),
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn fetch(
        &self,
        anchor: &AbsoluteSystemPath,
        key: &str,
    ) -> Result<Option<(CacheHitMetadata, Vec<AnchoredSystemPathBuf>)>, CacheError> {
        if self.cache_config.local.read
            && let Some(fs) = &self.fs
        {
            // Checking whether the recorded outputs are still on disk only
            // reads metadata (the manifest and one stat per output), small
            // synchronous I/O of the kind task log replay already does in
            // place, so it runs here. Most cache hits end at this check, and
            // doing it in place saves two thread hand-offs per task. Manifests
            // over the limit are checked on the blocking pool so a huge output
            // set can't stall a worker.
            let response = match fs.lookup(anchor, key, MAX_IN_PLACE_MANIFEST_BYTES) {
                Ok(LocalLookup::Miss) => Ok(None),
                Ok(LocalLookup::Unchanged(metadata, files)) => Ok(Some((metadata, files))),
                // Local restore (decompression + extraction) is synchronous;
                // keep it off the runtime workers.
                Ok(LocalLookup::Restore(previous_manifest)) => {
                    let fs = fs.clone();
                    let anchor = anchor.to_owned();
                    let key = key.to_owned();
                    tokio::task::spawn_blocking(move || {
                        fs.restore(&anchor, &key, previous_manifest)
                    })
                    .await?
                }
                Ok(LocalLookup::ManifestTooLarge) => {
                    let fs = fs.clone();
                    let anchor = anchor.to_owned();
                    let key = key.to_owned();
                    tokio::task::spawn_blocking(move || fs.fetch(&anchor, &key)).await?
                }
                Err(error) => Err(error),
            };
            if let response @ Ok(Some(_)) = response {
                return response;
            }
            // Ok(None) or Err: fall through to the remote cache, matching
            // previous behavior.
        }

        if self.cache_config.remote.read
            && let Some(http) = self.get_http_cache()
        {
            // When the remote hit is also written to the local cache, install
            // the verified archive bytes directly instead of restoring and
            // then re-reading/re-compressing every output file.
            if self.cache_config.local.write
                && let Some(fs) = &self.fs
            {
                if let Ok(Some((hit_metadata, files, artifact))) =
                    http.fetch_with_archive(key).await
                {
                    // We can ignore errors here because we know we have
                    // previously successfully fetched from the HTTP cache, and
                    // so the overall result is a success. Storing in
                    // lower-priority caches is an optimization. The archive
                    // passed signature verification before any restore ran, so
                    // a rejected download never becomes a local hit.
                    let time_saved = hit_metadata.time_saved;
                    let _ = {
                        let fs = fs.clone();
                        let key = key.to_owned();
                        tokio::task::spawn_blocking(move || {
                            fs.put_archive(&key, &artifact, time_saved)
                        })
                        .await
                    };
                    return Ok(Some((hit_metadata, files)));
                }
            } else if let Ok(Some((hit_metadata, files))) = http.fetch(key).await {
                return Ok(Some((hit_metadata, files)));
            }
        }

        Ok(None)
    }

    /// Starts downloading the remote artifacts for `keys` (in order) ahead of
    /// the tasks that will fetch them, so remote round trips overlap instead
    /// of waiting for each task's dependencies. `keys` is only called when the
    /// remote cache is readable. Keys with a local cache entry are skipped,
    /// since `fetch` restores those locally.
    pub fn prefetch_remote(&self, keys: impl FnOnce() -> Vec<String>, concurrency: usize) {
        if !self.cache_config.remote.read {
            return;
        }
        let Some(http) = self.get_http_cache() else {
            return;
        };
        let fs = self.fs.clone().filter(|_| self.cache_config.local.read);
        http.prefetch(keys(), concurrency, move |key| {
            fs.as_ref()
                .is_some_and(|fs| matches!(fs.exists(key), Ok(Some(_))))
        });
    }

    /// Whether [`Self::batch_exists`] could ever consult the remote cache.
    /// When this is false a batch reduces to independent local lookups, so
    /// there is nothing to gain from waiting for other keys.
    pub(crate) fn may_read_remote(&self) -> bool {
        self.cache_config.remote.read && self.http.is_some()
    }

    /// The local half of [`Self::batch_exists`] for a single key.
    pub(crate) fn local_exists(&self, key: &str) -> Option<CacheHitMetadata> {
        if self.cache_config.local.read
            && let Some(fs) = &self.fs
            && let Ok(Some(hit)) = fs.exists(key)
        {
            Some(hit)
        } else {
            None
        }
    }

    /// Dry-run-only batched existence check. Local hits take precedence; a
    /// server without the query endpoint still works through individual HEADs.
    pub async fn batch_exists(&self, keys: &[String]) -> Vec<Option<CacheHitMetadata>> {
        let mut hits = vec![None; keys.len()];
        let mut remote_keys = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            if self.cache_config.local.read
                && let Some(fs) = &self.fs
                && let Ok(Some(hit)) = fs.exists(key)
            {
                hits[index] = Some(hit);
            } else {
                remote_keys.push((index, key.clone()));
            }
        }
        if remote_keys.is_empty() || !self.cache_config.remote.read {
            return hits;
        }
        let Some(http) = self.get_http_cache() else {
            return hits;
        };
        let hashes: Vec<_> = remote_keys.iter().map(|(_, key)| key.clone()).collect();
        let batch = http.batch_exists(&hashes).await;
        // Older self-hosted caches do not implement the batch endpoint. Resolve
        // their HEAD fallbacks concurrently so network latency does not add up
        // once per task, while limiting the number of in-flight requests.
        let batch = &batch;
        let resolved = stream::iter(remote_keys.into_iter().map(|(index, key)| async move {
            let hit = match batch {
                Ok(entries) => match entries.get(&key) {
                    Some(hit) => hit.clone(),
                    None => http.exists(&key).await.ok().flatten(),
                },
                Err(_) => http.exists(&key).await.ok().flatten(),
            };
            (index, hit)
        }))
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
        for (index, hit) in resolved {
            hits[index] = hit;
        }
        hits
    }

    #[tracing::instrument(skip_all)]
    pub async fn exists(&self, key: &str) -> Result<Option<CacheHitMetadata>, CacheError> {
        if self.cache_config.local.read
            && let Some(fs) = &self.fs
        {
            match fs.exists(key) {
                cache_hit @ Ok(Some(_)) => {
                    return cache_hit;
                }
                Ok(None) => {}
                Err(err) => debug!("failed to check fs cache: {:?}", err),
            }
        }

        if self.cache_config.remote.read
            && let Some(http) = self.get_http_cache()
        {
            match http.exists(key).await {
                cache_hit @ Ok(Some(_)) => {
                    return cache_hit;
                }
                Ok(None) => {}
                Err(err) => debug!("failed to check http cache: {:?}", err),
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_api_client::{APIAuth, APIClient};
    use turborepo_types::SecretString;
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::{CacheActions, CacheSource, RemoteCacheOpts};

    fn both_write_opts() -> CacheOpts {
        CacheOpts {
            cache_dir: ".turbo/cache".into(),
            cache: CacheConfig {
                local: CacheActions {
                    read: true,
                    write: true,
                },
                remote: CacheActions {
                    read: true,
                    write: true,
                },
            },
            workers: 0,
            remote_cache_opts: Some(RemoteCacheOpts {
                unused_team_id: Some("my-team".to_string()),
                signature: false,
                enforce_signature_key_length: false,
            }),
            cache_max_age: None,
            cache_max_size: None,
        }
    }

    fn test_multiplexer(
        opts: &CacheOpts,
        repo_root: &AbsoluteSystemPathBuf,
        port: u16,
    ) -> CacheMultiplexer {
        CacheMultiplexer::new(
            opts,
            repo_root,
            Some(
                APIClient::new(
                    format!("http://localhost:{port}"),
                    Some(Duration::from_secs(200)),
                    None,
                    "2.0.0",
                    true,
                )
                .unwrap(),
            ),
            Some(APIAuth {
                team_id: Some("my-team".to_string()),
                token: SecretString::new("my-token".to_string()),
                team_slug: None,
            }),
            None,
            LazyScmState::resolved(None),
        )
        .unwrap()
    }

    async fn start_mock() -> (u16, tokio::task::JoinHandle<Result<()>>) {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("test server start timed out")
            .expect("test server failed to start");
        (port, handle)
    }

    async fn remote_bytes(port: u16, hash: &str) -> Vec<u8> {
        reqwest::get(format!("http://localhost:{port}/v8/artifacts/{hash}"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn batch_exists_prefers_local_hit_without_querying_remote() -> Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let query = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/v8/artifacts");
                then.status(500);
            })
            .await;
        let root = tempdir()?;
        let root = AbsoluteSystemPathBuf::try_from(root.path())?;
        let file = AnchoredSystemPathBuf::from_raw("output.txt")?;
        std::fs::write(root.resolve(&file), "local content")?;
        let cache = CacheMultiplexer::new(
            &both_write_opts(),
            &root,
            Some(APIClient::new(
                server.base_url(),
                None,
                None,
                "2.0.0",
                false,
            )?),
            Some(APIAuth {
                team_id: None,
                team_slug: None,
                token: SecretString::new("token".into()),
            }),
            None,
            LazyScmState::resolved(None),
        )?;
        cache
            .fs
            .as_ref()
            .unwrap()
            .put(&root, "local", &[file], 17)?;
        let hits = cache.batch_exists(&["local".into()]).await;
        assert_eq!(
            hits,
            vec![Some(CacheHitMetadata {
                source: CacheSource::Local,
                time_saved: 17,
                sha: None,
                dirty_hash: None,
            })]
        );
        query.assert_calls_async(0).await;
        Ok(())
    }

    fn mock_server_multiplexer(
        opts: &CacheOpts,
        root: &AbsoluteSystemPathBuf,
        server: &httpmock::MockServer,
    ) -> Result<CacheMultiplexer> {
        Ok(CacheMultiplexer::new(
            opts,
            root,
            Some(APIClient::new(
                server.base_url(),
                None,
                None,
                "2.0.0",
                false,
            )?),
            Some(APIAuth {
                team_id: None,
                team_slug: None,
                token: SecretString::new("token".into()),
            }),
            None,
            LazyScmState::resolved(None),
        )?)
    }

    async fn mock_remote_hit(server: &httpmock::MockServer) -> httpmock::Mock<'_> {
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::HEAD)
                    .path("/v8/artifacts/remote-hit");
                then.status(200).header("x-artifact-duration", "7");
            })
            .await
    }

    #[tokio::test]
    async fn disable_remote_writes_skips_uploads_and_respects_read_only_config() -> Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let put = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::PUT);
                then.status(202);
            })
            .await;
        let remote_hit = mock_remote_hit(&server).await;

        let root = tempdir()?;
        let root = AbsoluteSystemPathBuf::try_from(root.path())?;
        let file = AnchoredSystemPathBuf::from_raw("output.txt")?;
        std::fs::write(root.resolve(&file), "content")?;
        let cache = mock_server_multiplexer(&both_write_opts(), &root, &server)?;
        cache.disable_remote_writes();
        cache
            .put(&root, "status-read-only", std::slice::from_ref(&file), 1)
            .await?;
        put.assert_calls_async(0).await;
        assert_eq!(
            cache.exists("remote-hit").await?.unwrap().source,
            CacheSource::Remote
        );
        remote_hit.assert_calls_async(1).await;

        // An explicit `remote:r` is already read-only and stays untouched.
        let mut read_only_opts = both_write_opts();
        read_only_opts.cache.remote.write = false;
        let cache = mock_server_multiplexer(&read_only_opts, &root, &server)?;
        cache.disable_remote_writes();
        assert!(!cache.http.as_ref().unwrap().uploads_disabled());
        Ok(())
    }

    #[tokio::test]
    async fn test_disabled_remote_cache_preserves_local_reads_and_writes() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        let file = AnchoredSystemPathBuf::from_raw("out/output.txt")?;
        std::fs::create_dir_all(repo_root_path.resolve(&file).parent().unwrap())?;
        std::fs::write(repo_root_path.resolve(&file), "local fallback")?;

        // A failed 403 recovery disables the HTTP cache, even though remote
        // reads and writes remain enabled in the original configuration.
        let cache = test_multiplexer(&both_write_opts(), &repo_root_path, 1);
        cache.http.as_ref().unwrap().disable_after_forbidden();
        assert!(cache.get_http_cache().is_none());

        cache
            .put(
                &repo_root_path,
                "local-only",
                std::slice::from_ref(&file),
                42,
            )
            .await?;
        assert_eq!(
            cache.exists("local-only").await?.unwrap().source,
            CacheSource::Local
        );
        std::fs::remove_file(repo_root_path.resolve(&file))?;
        assert_eq!(
            cache
                .fetch(&repo_root_path, "local-only")
                .await?
                .unwrap()
                .0
                .source,
            CacheSource::Local
        );
        assert_eq!(
            std::fs::read(repo_root_path.resolve(&file))?,
            b"local fallback"
        );
        assert!(cache.exists("remote-only").await?.is_none());
        assert_eq!(
            cache.batch_exists(&["remote-only".into()]).await,
            vec![None]
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_outage_preserves_local_but_remote_only_write_reports_failure() -> Result<()> {
        let repo_root = tempdir()?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        let file = AnchoredSystemPathBuf::from_raw("out/output.txt")?;
        std::fs::create_dir_all(root.resolve(&file).parent().unwrap())?;
        std::fs::write(root.resolve(&file), "local fallback")?;

        let cache = test_multiplexer(&both_write_opts(), &root, 1);
        cache.http.as_ref().unwrap().trip_outage_for_test();
        cache
            .put(&root, "local-fallback", std::slice::from_ref(&file), 42)
            .await?;
        assert_eq!(
            cache.exists("local-fallback").await?.unwrap().source,
            CacheSource::Local
        );
        std::fs::remove_file(root.resolve(&file))?;
        assert_eq!(
            cache
                .fetch(&root, "local-fallback")
                .await?
                .unwrap()
                .0
                .source,
            CacheSource::Local
        );

        let mut remote_opts = both_write_opts();
        remote_opts.cache.local = CacheActions {
            read: false,
            write: false,
        };
        let remote_only = test_multiplexer(&remote_opts, &root, 1);
        remote_only.http.as_ref().unwrap().trip_outage_for_test();
        assert!(matches!(
            remote_only.put(&root, "remote-only", &[], 42).await,
            Err(CacheError::RemoteCacheUnavailable)
        ));
        Ok(())
    }

    /// With local and remote writes both enabled, one canonical archive must
    /// be built: the bytes installed locally are exactly the bytes uploaded.
    #[tokio::test]
    async fn test_put_builds_one_archive_for_local_and_remote() -> Result<()> {
        let (port, handle) = start_mock().await;

        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        let file = AnchoredSystemPathBuf::from_raw("out/output.txt")?;
        std::fs::create_dir_all(repo_root_path.resolve(&file).parent().unwrap())?;
        std::fs::write(repo_root_path.resolve(&file), "shared archive contents")?;

        let hash = "shared-put-hash";
        let cache = test_multiplexer(&both_write_opts(), &repo_root_path, port);
        cache
            .put(&repo_root_path, hash, std::slice::from_ref(&file), 42)
            .await?;

        let local_bytes = std::fs::read(repo_root_path.join_components(&[
            ".turbo",
            "cache",
            &format!("{hash}.tar.zst"),
        ]))?;
        let uploaded = remote_bytes(port, hash).await;

        assert!(!uploaded.is_empty());
        assert_eq!(
            local_bytes, uploaded,
            "local and remote destinations must contain the same archive bytes"
        );

        handle.abort();
        Ok(())
    }

    /// A remote hit with local writes enabled installs the verified remote
    /// archive bytes locally instead of re-encoding restored files.
    #[tokio::test]
    async fn test_fetch_installs_remote_archive_bytes() -> Result<()> {
        let (port, handle) = start_mock().await;

        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        let file = AnchoredSystemPathBuf::from_raw("out/output.txt")?;
        std::fs::create_dir_all(repo_root_path.resolve(&file).parent().unwrap())?;
        std::fs::write(repo_root_path.resolve(&file), "fetch install contents")?;

        let hash = "shared-fetch-hash";
        let cache = test_multiplexer(&both_write_opts(), &repo_root_path, port);
        cache
            .put(&repo_root_path, hash, std::slice::from_ref(&file), 42)
            .await?;

        let uploaded = remote_bytes(port, hash).await;

        // Remove the local archive and the outputs so the fetch must be a
        // remote hit that restores and re-installs.
        std::fs::remove_file(repo_root_path.join_components(&[
            ".turbo",
            "cache",
            &format!("{hash}.tar.zst"),
        ]))?;
        std::fs::remove_file(repo_root_path.resolve(&file))?;

        let (metadata, files) = cache
            .fetch(&repo_root_path, hash)
            .await?
            .expect("remote hit expected");
        assert_eq!(metadata.source, CacheSource::Remote);
        assert_eq!(files, vec![file.clone()]);
        assert_eq!(
            std::fs::read(repo_root_path.resolve(&file))?,
            b"fetch install contents"
        );

        let local_bytes = std::fs::read(repo_root_path.join_components(&[
            ".turbo",
            "cache",
            &format!("{hash}.tar.zst"),
        ]))?;
        assert_eq!(
            local_bytes, uploaded,
            "locally installed archive must be the downloaded bytes, not a re-encode"
        );

        handle.abort();
        Ok(())
    }
}
