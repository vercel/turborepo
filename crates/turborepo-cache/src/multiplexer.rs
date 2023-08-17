use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_api_client::APIClient;

use crate::{
    fs::FSCache,
    http::{APIAuth, HTTPCache},
    CacheError, CacheOpts, CacheResponse,
};

pub struct CacheMultiplexer {
    // We use an `AtomicBool` instead of removing the cache because that would require
    // wrapping the cache in a `Mutex` which would cause a lot of contention.
    // This does create a mild race condition where we might use the cache
    // even though another thread might be removing it, but that's fine.
    should_use_http_cache: AtomicBool,
    fs: Option<FSCache>,
    http: Option<HTTPCache>,
}

impl CacheMultiplexer {
    pub fn new(
        opts: &CacheOpts,
        repo_root: &AbsoluteSystemPath,
        api_client: APIClient,
        api_auth: Option<APIAuth>,
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
            .then(|| FSCache::new(opts.override_dir, repo_root))
            .transpose()?;

        let http_cache = use_http_cache
            .then_some(api_auth)
            .flatten()
            .map(|api_auth| HTTPCache::new(api_client, opts, repo_root.to_owned(), api_auth));

        Ok(CacheMultiplexer {
            should_use_http_cache: AtomicBool::new(http_cache.is_some()),
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
                let http_result = http.put(anchor, key, files, duration).await;

                Some(http_result)
            }
            _ => None,
        };

        if let Some(Err(CacheError::ApiClientError(
            box turborepo_api_client::Error::CacheDisabled { .. },
            ..,
        ))) = http_result
        {
            warn!("failed to put to http cache: cache disabled");
            self.should_use_http_cache.store(false, Ordering::Relaxed);
        }

        Ok(())
    }

    pub async fn fetch(
        &self,
        anchor: &AbsoluteSystemPath,
        key: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<(CacheResponse, Vec<AnchoredSystemPathBuf>), CacheError> {
        if let Some(fs) = &self.fs {
            if let Ok(cache_response) = fs.fetch(anchor, key) {
                return Ok(cache_response);
            }
        }

        if let Some(http) = self.get_http_cache() {
            if let Ok((cache_response, files)) = http.fetch(key, team_id, team_slug).await {
                // Store this into fs cache. We can ignore errors here because we know
                // we have previously successfully stored in HTTP cache, and so the overall
                // result is a success at fetching. Storing in lower-priority caches is an
                // optimization.
                if let Some(fs) = &self.fs {
                    let _ = fs.put(anchor, key, &files, cache_response.time_saved);
                }

                return Ok((cache_response, files));
            }
        }

        Err(CacheError::CacheMiss)
    }

    pub async fn exists(
        &self,
        key: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<CacheResponse, CacheError> {
        if let Some(fs) = &self.fs {
            match fs.exists(key) {
                Ok(cache_response) => {
                    return Ok(cache_response);
                }
                Err(err) => debug!("failed to check fs cache: {:?}", err),
            }
        }

        if let Some(http) = self.get_http_cache() {
            match http.exists(key, team_id, team_slug).await {
                Ok(cache_response) => {
                    return Ok(cache_response);
                }
                Err(err) => debug!("failed to check http cache: {:?}", err),
            }
        }

        Err(CacheError::CacheMiss)
    }
}
