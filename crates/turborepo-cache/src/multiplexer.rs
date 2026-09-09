use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use tracing::{debug, info, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{APIAuth, APIClient};

use crate::{
    CacheConfig, CacheError, CacheHitMetadata, CacheOpts, LazyScmState,
    fs::FSCache,
    http::{HTTPCache, UploadMap},
};

pub struct CacheMultiplexer {
    // We use an `AtomicBool` instead of removing the cache because that would require
    // wrapping the cache in a `Mutex` which would cause a lot of contention.
    // This does create a mild race condition where we might use the cache
    // even though another thread might be removing it, but that's fine.
    should_use_http_cache: AtomicBool,
    // Just for keeping track of whether we've already printed a warning about the remote cache
    // being read-only
    should_print_skipping_remote_put: AtomicBool,
    cache_config: CacheConfig,
    fs: Option<FSCache>,
    http: Option<HTTPCache>,
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
                (Some(api_client), Some(api_auth)) => Some(HTTPCache::new(
                    api_client,
                    opts,
                    repo_root.to_owned(),
                    api_auth,
                    analytics_recorder.clone(),
                    scm_state.clone(),
                )?),
                _ => None,
            }
        } else {
            None
        };

        Ok(CacheMultiplexer {
            should_print_skipping_remote_put: AtomicBool::new(true),
            should_use_http_cache: AtomicBool::new(http_cache.is_some()),
            cache_config: opts.cache,
            fs: fs_cache,
            http: http_cache,
            scm_state,
        })
    }

    // This is technically a TOCTOU bug, but at worst it'll cause
    // a few extra cache requests.
    fn get_http_cache(&self) -> Option<&HTTPCache> {
        if self.should_use_http_cache.load(Ordering::Relaxed) {
            self.http.as_ref()
        } else {
            None
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
        {
            let body = crate::artifact_body::ArtifactBody::from_files(anchor, files)?;

            fs.put_archive(anchor, key, files, &body, duration)?;
            let http_result = http.put_body(key, body, duration).await;

            return match http_result {
                Err(CacheError::ApiClientError(
                    box turborepo_api_client::Error::CacheDisabled { .. },
                    ..,
                )) => {
                    warn!("failed to put to http cache: cache disabled");
                    self.should_use_http_cache.store(false, Ordering::Relaxed);
                    Ok(())
                }
                Err(e) => Err(e),
                Ok(()) => Ok(()),
            };
        }

        if self.cache_config.local.write {
            self.fs
                .as_ref()
                .map(|fs| fs.put(anchor, key, files, duration))
                .transpose()?;
        }

        let http_result = match self.get_http_cache() {
            Some(http) => {
                if self.cache_config.remote.write {
                    let http_result = http.put(anchor, key, files, duration).await;

                    Some(http_result)
                } else {
                    if self
                        .should_print_skipping_remote_put
                        .load(Ordering::Relaxed)
                    {
                        turborepo_log::warn(
                            turborepo_log::Source::turbo(turborepo_log::Subsystem::Cache),
                            "Remote cache is read-only, skipping upload",
                        )
                        .emit();
                        self.should_print_skipping_remote_put
                            .store(false, Ordering::Relaxed);
                    }
                    // Cache is functional but running in read-only mode, so we don't want to try to
                    // write to it
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
            && let response @ Ok(Some(_)) = fs.fetch(anchor, key)
        {
            return response;
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
                if let Ok(Some((hit_metadata, files, body))) = http.fetch_with_archive(key).await {
                    // We can ignore errors here because we know we have
                    // previously successfully fetched from the HTTP cache, and
                    // so the overall result is a success. Storing in
                    // lower-priority caches is an optimization. The archive
                    // passed signature verification before any restore ran, so
                    // a rejected download never becomes a local hit.
                    let _ = fs.put_archive(anchor, key, &files, &body, hit_metadata.time_saved);
                    return Ok(Some((hit_metadata, files)));
                }
            } else if let Ok(Some((hit_metadata, files))) = http.fetch(key).await {
                return Ok(Some((hit_metadata, files)));
            }
        }

        Ok(None)
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
        let port = port_scanner::request_open_port().unwrap();
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

        let local_bytes = std::fs::read(
            repo_root_path.join_components(&[".turbo", "cache", &format!("{hash}.tar.zst")]),
        )?;
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
        std::fs::remove_file(
            repo_root_path.join_components(&[".turbo", "cache", &format!("{hash}.tar.zst")]),
        )?;
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

        let local_bytes = std::fs::read(
            repo_root_path.join_components(&[".turbo", "cache", &format!("{hash}.tar.zst")]),
        )?;
        assert_eq!(
            local_bytes, uploaded,
            "locally installed archive must be the downloaded bytes, not a re-encode"
        );

        handle.abort();
        Ok(())
    }
}
