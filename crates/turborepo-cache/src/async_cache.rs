use std::sync::Arc;

use futures::{stream::FuturesUnordered, StreamExt};
use tokio::task::JoinHandle;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_api_client::APIClient;

use crate::{http::APIAuth, multiplexer::CacheMultiplexer, CacheError, CacheOpts, CacheResponse};

pub struct AsyncCache {
    workers: FuturesUnordered<JoinHandle<()>>,
    max_workers: usize,
    real_cache: Arc<CacheMultiplexer>,
}

impl AsyncCache {
    pub fn new(
        opts: &CacheOpts,
        repo_root: &AbsoluteSystemPath,
        api_client: APIClient,
        api_auth: Option<APIAuth>,
    ) -> Result<AsyncCache, CacheError> {
        let max_workers = opts.workers.try_into().expect("usize is smaller than u32");
        let real_cache = CacheMultiplexer::new(opts, repo_root, api_client, api_auth)?;

        Ok(AsyncCache {
            workers: FuturesUnordered::new(),
            real_cache: Arc::new(real_cache),
            max_workers,
        })
    }

    pub async fn put(
        &mut self,
        anchor: AbsoluteSystemPathBuf,
        key: String,
        files: Vec<AnchoredSystemPathBuf>,
        duration: u32,
    ) {
        if self.workers.len() >= self.max_workers {
            let _ = self.workers.next().await.unwrap();
        }

        let real_cache = self.real_cache.clone();

        let fut = tokio::spawn(async move {
            let _ = real_cache.put(&anchor, &key, &files, duration).await;
        });
        self.workers.push(fut);
    }

    pub async fn fetch(
        &self,
        anchor: &AbsoluteSystemPath,
        key: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<(CacheResponse, Vec<AnchoredSystemPathBuf>), CacheError> {
        self.real_cache.fetch(anchor, key, team_id, team_slug).await
    }

    pub async fn exists(
        &mut self,
        key: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<CacheResponse, CacheError> {
        self.real_cache.exists(key, team_id, team_slug).await
    }

    // Used for testing to ensure that the workers resolve
    // before checking the cache.
    #[cfg(test)]
    pub async fn wait(&mut self) {
        while let Some(worker) = self.workers.next().await {
            let _ = worker;
        }
    }

    pub async fn shutdown(self) {
        for worker in self.workers {
            let _ = worker.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use anyhow::Result;
    use futures::future::try_join_all;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_api_client::APIClient;
    use vercel_api_mock::start_test_server;

    use crate::{
        http::APIAuth,
        test_cases::{get_test_cases, TestCase},
        AsyncCache, CacheError, CacheOpts, CacheResponse, CacheSource, RemoteCacheOpts,
    };

    #[tokio::test]
    async fn test_async_cache() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));

        try_join_all(get_test_cases().into_iter().map(|test_case| async {
            let test_case = test_case;
            round_trip_test_with_both_caches(&test_case, port).await?;
            round_trip_test_without_remote_cache(&test_case).await?;
            round_trip_test_without_fs(&test_case, port).await
        }))
        .await?;

        handle.abort();
        Ok(())
    }

    async fn round_trip_test_without_fs(test_case: &TestCase, port: u16) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let hash = format!("{}-no-fs", test_case.hash);

        let opts = CacheOpts {
            override_dir: None,
            skip_remote: false,
            skip_filesystem: true,
            workers: 10,
            remote_cache_opts: Some(RemoteCacheOpts {
                team_id: "my-team".to_string(),
                signature: false,
            }),
        };

        let api_client = APIClient::new(format!("http://localhost:{}", port), 200, "2.0.0", true)?;
        let api_auth = Some(APIAuth {
            team_id: "my-team-id".to_string(),
            token: "my-token".to_string(),
        });
        let mut async_cache = AsyncCache::new(&opts, &repo_root_path, api_client, api_auth)?;

        // Ensure that the cache is empty
        let response = async_cache.exists(&hash, "my-team-id", None).await;

        assert_matches!(response, Err(CacheError::CacheMiss));

        // Add test case
        async_cache
            .put(
                repo_root_path.clone(),
                hash.clone(),
                test_case.files.iter().map(|f| f.path.clone()).collect(),
                test_case.duration,
            )
            .await;

        // Wait for async cache to process
        async_cache.wait().await;

        let fs_cache_path = repo_root_path.join_components(&[
            "node_modules",
            ".cache",
            "turbo",
            &format!("{}.tar.zst", hash),
        ]);

        // Confirm that fs cache file does *not* exist
        assert!(!fs_cache_path.exists());

        let response = async_cache.exists(&hash, "my-team-id", None).await?;

        // Confirm that we fetch from remote cache and not local.
        assert_eq!(
            response,
            CacheResponse {
                source: CacheSource::Remote,
                time_saved: test_case.duration
            }
        );

        Ok(())
    }

    async fn round_trip_test_without_remote_cache(test_case: &TestCase) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let hash = format!("{}-no-remote", test_case.hash);

        let opts = CacheOpts {
            override_dir: None,
            skip_remote: true,
            skip_filesystem: false,
            workers: 10,
            remote_cache_opts: Some(RemoteCacheOpts {
                team_id: "my-team".to_string(),
                signature: false,
            }),
        };

        // Initialize client with invalid API url to ensure that we don't hit the
        // network
        let api_client = APIClient::new("http://example.com", 200, "2.0.0", true)?;
        let api_auth = Some(APIAuth {
            team_id: "my-team-id".to_string(),
            token: "my-token".to_string(),
        });
        let mut async_cache = AsyncCache::new(&opts, &repo_root_path, api_client, api_auth)?;

        // Ensure that the cache is empty
        let response = async_cache.exists(&hash, "my-team-id", None).await;

        assert_matches!(response, Err(CacheError::CacheMiss));

        // Add test case
        async_cache
            .put(
                repo_root_path.clone(),
                hash.clone(),
                test_case.files.iter().map(|f| f.path.clone()).collect(),
                test_case.duration,
            )
            .await;

        // Wait for async cache to process
        async_cache.wait().await;

        let fs_cache_path = repo_root_path.join_components(&[
            "node_modules",
            ".cache",
            "turbo",
            &format!("{}.tar.zst", hash),
        ]);

        // Confirm that fs cache file exists
        assert!(fs_cache_path.exists());

        let response = async_cache.exists(&hash, "my-team-id", None).await?;

        // Confirm that we fetch from local cache first.
        assert_eq!(
            response,
            CacheResponse {
                source: CacheSource::Local,
                time_saved: test_case.duration
            }
        );

        // Remove fs cache file
        fs_cache_path.remove_file()?;

        let response = async_cache.exists(&hash, "my-team-id", None).await;

        // Confirm that we get a cache miss
        assert_matches!(response, Err(CacheError::CacheMiss));

        Ok(())
    }

    async fn round_trip_test_with_both_caches(test_case: &TestCase, port: u16) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let hash = format!("{}-both", test_case.hash);

        let opts = CacheOpts {
            override_dir: None,
            skip_remote: false,
            skip_filesystem: false,
            workers: 10,
            remote_cache_opts: Some(RemoteCacheOpts {
                team_id: "my-team".to_string(),
                signature: false,
            }),
        };

        let api_client = APIClient::new(format!("http://localhost:{}", port), 200, "2.0.0", true)?;
        let api_auth = Some(APIAuth {
            team_id: "my-team-id".to_string(),
            token: "my-token".to_string(),
        });
        let mut async_cache = AsyncCache::new(&opts, &repo_root_path, api_client, api_auth)?;

        // Ensure that the cache is empty
        let response = async_cache.exists(&hash, "my-team-id", None).await;

        assert_matches!(response, Err(CacheError::CacheMiss));

        // Add test case
        async_cache
            .put(
                repo_root_path.clone(),
                hash.clone(),
                test_case.files.iter().map(|f| f.path.clone()).collect(),
                test_case.duration,
            )
            .await;

        // Wait for async cache to process
        async_cache.wait().await;

        let fs_cache_path = repo_root_path.join_components(&[
            "node_modules",
            ".cache",
            "turbo",
            &format!("{}.tar.zst", hash),
        ]);

        // Confirm that fs cache file exists
        assert!(fs_cache_path.exists());

        let response = async_cache.exists(&hash, "my-team-id", None).await?;

        // Confirm that we fetch from local cache first.
        assert_eq!(
            response,
            CacheResponse {
                source: CacheSource::Local,
                time_saved: test_case.duration
            }
        );

        // Remove fs cache file
        fs_cache_path.remove_file()?;

        let response = async_cache.exists(&hash, "my-team-id", None).await?;

        // Confirm that we still can fetch from remote cache
        assert_eq!(
            response,
            CacheResponse {
                source: CacheSource::Remote,
                time_saved: test_case.duration
            }
        );

        Ok(())
    }
}
