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
            && let Ok(Some((hit_metadata, files))) = http.fetch(key).await
        {
            // Store this into fs cache. We can ignore errors here because we know
            // we have previously successfully stored in HTTP cache, and so the overall
            // result is a success at fetching. Storing in lower-priority caches is an
            // optimization.
            if self.cache_config.local.write
                && let Some(fs) = &self.fs
            {
                let _ = fs.put(anchor, key, &files, hit_metadata.time_saved);
            }

            return Ok(Some((hit_metadata, files)));
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
