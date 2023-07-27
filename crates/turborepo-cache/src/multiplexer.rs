use camino::Utf8Path;
use futures::future::join;
use tracing::warn;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_api_client::APIClient;

use crate::{fs::FsCache, http::HttpCache, CacheError, CacheOpts};

pub struct CacheMultiplexer {
    fs: Option<FsCache>,
    http: Option<HttpCache>,
}

impl CacheMultiplexer {
    pub fn new(
        opts: CacheOpts,
        override_dir: Option<&Utf8Path>,
        repo_root: &AbsoluteSystemPath,
        api_client: APIClient,
        team_id: &str,
        token: &str,
    ) -> Result<Self, CacheError> {
        let use_fs_cache = !opts.skip_filesystem;
        let use_http_cache = !opts.skip_remote;
        // Since the above two flags are not mutually exclusive it is possible to
        // configure yourself out of having a cache. We should tell you about it
        // but we shouldn't fail your build for that reason.
        //
        // Further, since the httpCache can be removed at runtime, we need to insert a
        // noopCache as a backup if you are configured to have *just* an
        // httpCache.
        //
        if !use_fs_cache && !use_http_cache {
            warn!("no caches are enabled");
        }

        let fs_cache = use_fs_cache
            .then(|| FsCache::new(override_dir, repo_root))
            .transpose()?;

        let http_cache = use_http_cache
            .then(|| HttpCache::new(api_client, opts, repo_root.to_owned(), team_id, token));

        Ok(CacheMultiplexer {
            fs: fs_cache,
            http: http_cache,
        })
    }

    pub async fn put(
        &mut self,
        anchor: &AbsoluteSystemPath,
        key: &str,
        files: Vec<AnchoredSystemPathBuf>,
        duration: u32,
        token: &str,
    ) -> Result<(), CacheError> {
        let (http_result, fs_result) = match (&self.http, &self.fs) {
            (Some(http), Some(fs)) => {
                let (http_result, fs_result) = join(
                    http.put(anchor, key, &files, duration, token),
                    fs.put(anchor, key, &files, duration),
                )
                .await;

                (Some(http_result), Some(fs_result))
            }
            (None, Some(fs)) => {
                let fs_result = fs.put(anchor, key, &files, duration).await;
                (None, Some(fs_result))
            }
            (Some(http), None) => {
                let http_result = http.put(anchor, key, &files, duration, token).await;
                (Some(http_result), None)
            }
            (None, None) => return Ok(()),
        };

        if let Some(Err(http_err)) = http_result {
            if let Err(CacheError::ApiClientError(
                box turborepo_api_client::Error::CacheDisabled { .. },
                ..,
            )) = http_err
            {
                warn!("failed to put to http cache: cache disabled");
                self.http = None;
            }
        }

        Ok(())
    }
}
