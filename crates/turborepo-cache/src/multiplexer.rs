use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use tracing::{debug, warn, error};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{APIAuth, APIClient};

use crate::{
    CacheConfig, CacheError, CacheHitMetadata, CacheOpts,
    fs::FSCache,
    http::{HTTPCache, UploadMap},
};

pub struct CacheMultiplexer {
    // We use an `AtomicBool` instead of removing the cache because that would require
    // wrapping the cache in a `Mutex` which would cause a lot of contention.
    // This does create a mild race condition where we might use the cache
    // even though another thread might be removing it, but that's fine.
    should_use_http_cache: AtomicBool,
    // Ensures we only show one connection error message for remote cache
    printed_remote_connect_error: AtomicBool,
    // Just for keeping track of whether we've already printed a warning about the remote cache
    // being read-only
    should_print_skipping_remote_put: AtomicBool,
    cache_config: CacheConfig,
    fs: Option<FSCache>,
    http: Option<HTTPCache>,
}

impl CacheMultiplexer {
    #[tracing::instrument(skip_all)]
    pub fn new(
        opts: &CacheOpts,
        repo_root: &AbsoluteSystemPath,
        api_client: APIClient,
        api_auth: Option<APIAuth>,
        analytics_recorder: Option<AnalyticsSender>,
    ) -> Result<Self, CacheError> {
        let use_fs_cache = opts.cache.local.should_use();
        let use_http_cache = opts.cache.remote.should_use();

        // Since the above two flags are not mutually exclusive it is possible to
        // configure yourself out of having a cache. We should tell you about it
        // but we shouldn't fail your build for that reason.
        if !use_fs_cache && !use_http_cache {
            warn!("no caches are enabled");
        }

        let fs_cache = use_fs_cache
            .then(|| FSCache::new(&opts.cache_dir, repo_root, analytics_recorder.clone()))
            .transpose()?;

        let http_cache = use_http_cache
            .then_some(api_auth)
            .flatten()
            .map(|api_auth| {
                HTTPCache::new(
                    api_client,
                    opts,
                    repo_root.to_owned(),
                    api_auth,
                    analytics_recorder.clone(),
                )
            });

        Ok(CacheMultiplexer {
            should_print_skipping_remote_put: AtomicBool::new(true),
            should_use_http_cache: AtomicBool::new(http_cache.is_some()),
            printed_remote_connect_error: AtomicBool::new(false),
            cache_config: opts.cache,
            fs: fs_cache,
            http: http_cache,
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
                        // Warn once per build, not per task
                        warn!("Remote cache is read-only, skipping upload");
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
            match http.fetch(key).await {
                Ok(Some((CacheHitMetadata { source, time_saved }, files))) => {
                    // Store this into fs cache. We can ignore errors here because we know
                    // we have previously successfully stored in HTTP cache, and so the overall
                    // result is a success at fetching. Storing in lower-priority caches is an
                    // optimization.
                    if self.cache_config.local.write
                        && let Some(fs) = &self.fs
                    {
                        let _ = fs.put(anchor, key, &files, time_saved);
                    }

                    return Ok(Some((CacheHitMetadata { source, time_saved }, files)));
                }
                Ok(None) => { /* miss - fall through */ }
                Err(CacheError::ConnectError) => {
                    // Only print once per run to avoid noise across many workspaces
                    if !self.printed_remote_connect_error.swap(true, Ordering::Relaxed) {
                        error!(
                            "Cannot access remote cache (connection failed). Falling back to local cache if available."
                        );
                    }
                    // Disable further remote attempts for this run
                    self.should_use_http_cache.store(false, Ordering::Relaxed);
                }
                Err(CacheError::ApiClientError(..)) => {
                    // These are network/API level errors that indicate the remote cache is not accessible.
                    if !self.printed_remote_connect_error.swap(true, Ordering::Relaxed) {
                        error!(
                            "Cannot access remote cache (API error). Falling back to local cache if available."
                        );
                    }
                    self.should_use_http_cache.store(false, Ordering::Relaxed);
                }
                Err(other) => {
                    debug!("failed to fetch from http cache: {:?}", other);
                }
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
