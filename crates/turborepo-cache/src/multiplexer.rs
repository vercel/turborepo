use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{APIAuth, APIClient};

use crate::{
    fs::FSCache,
    http::{HTTPCache, UploadMap},
    CacheError, CacheHitMetadata, CacheOpts,
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
    remote_cache_read_only: bool,
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
        let use_fs_cache = !opts.skip_filesystem;
        let use_http_cache = !opts.skip_remote;

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
            remote_cache_read_only: opts.remote_cache_read_only,
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
        self.fs
            .as_ref()
            .map(|fs| fs.put(anchor, key, files, duration))
            .transpose()?;

        let http_result = match self.get_http_cache() {
            Some(http) => {
                if self.remote_cache_read_only {
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
                } else {
                    let http_result = http.put(anchor, key, files, duration).await;

                    Some(http_result)
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
        if let Some(fs) = &self.fs {
            if let response @ Ok(Some(_)) = fs.fetch(anchor, key) {
                return response;
            }
        }

        if let Some(http) = self.get_http_cache() {
            if let Ok(Some((CacheHitMetadata { source, time_saved }, files))) =
                http.fetch(key).await
            {
                // Store this into fs cache. We can ignore errors here because we know
                // we have previously successfully stored in HTTP cache, and so the overall
                // result is a success at fetching. Storing in lower-priority caches is an
                // optimization.
                if let Some(fs) = &self.fs {
                    let _ = fs.put(anchor, key, &files, time_saved);
                }

                return Ok(Some((CacheHitMetadata { source, time_saved }, files)));
            }
        }

        Ok(None)
    }

    #[tracing::instrument(skip_all)]
    pub async fn exists(&self, key: &str) -> Result<Option<CacheHitMetadata>, CacheError> {
        if let Some(fs) = &self.fs {
            match fs.exists(key) {
                cache_hit @ Ok(Some(_)) => {
                    return cache_hit;
                }
                Ok(None) => {}
                Err(err) => debug!("failed to check fs cache: {:?}", err),
            }
        }

        if let Some(http) = self.get_http_cache() {
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
