use std::sync::{atomic::AtomicU8, Arc};

use futures::{stream::FuturesUnordered, StreamExt};
use tokio::sync::{mpsc, Semaphore};
use tracing::{warn, Instrument, Level};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_analytics::AnalyticsSender;
use turborepo_api_client::{APIAuth, APIClient};

use crate::{multiplexer::CacheMultiplexer, CacheError, CacheHitMetadata, CacheOpts};

const WARNING_CUTOFF: u8 = 4;

#[derive(Clone)]
pub struct AsyncCache {
    real_cache: Arc<CacheMultiplexer>,
    writer_sender: mpsc::Sender<WorkerRequest>,
}

enum WorkerRequest {
    WriteRequest {
        anchor: AbsoluteSystemPathBuf,
        key: String,
        duration: u64,
        files: Vec<AnchoredSystemPathBuf>,
    },
    Flush(tokio::sync::oneshot::Sender<()>),
    Shutdown(tokio::sync::oneshot::Sender<()>),
}

impl AsyncCache {
    pub fn new(
        opts: &CacheOpts,
        repo_root: &AbsoluteSystemPath,
        api_client: APIClient,
        api_auth: Option<APIAuth>,
        analytics_recorder: Option<AnalyticsSender>,
    ) -> Result<AsyncCache, CacheError> {
        let max_workers = opts.workers.try_into().expect("usize is smaller than u32");
        let real_cache = Arc::new(CacheMultiplexer::new(
            opts,
            repo_root,
            api_client,
            api_auth,
            analytics_recorder,
        )?);
        let (writer_sender, mut write_consumer) = mpsc::channel(1);

        // start a task to manage workers
        let worker_real_cache = real_cache.clone();
        tokio::spawn(async move {
            let semaphore = Arc::new(Semaphore::new(max_workers));
            let mut workers = FuturesUnordered::new();
            let real_cache = worker_real_cache;
            let warnings = Arc::new(AtomicU8::new(0));

            let mut shutdown_callback = None;
            while let Some(request) = write_consumer.recv().await {
                match request {
                    WorkerRequest::WriteRequest {
                        anchor,
                        key,
                        duration,
                        files,
                    } => {
                        let permit = semaphore.clone().acquire_owned().await.unwrap();
                        let real_cache = real_cache.clone();
                        let warnings = warnings.clone();
                        let worker_span = tracing::span!(Level::TRACE, "cache worker: cache PUT");
                        workers.push(tokio::spawn(
                            async move {
                                if let Err(err) =
                                    real_cache.put(&anchor, &key, &files, duration).await
                                {
                                    let num_warnings =
                                        warnings.load(std::sync::atomic::Ordering::Acquire);
                                    if num_warnings <= WARNING_CUTOFF {
                                        warnings.store(
                                            num_warnings + 1,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                        warn!("{err}");
                                    }
                                }
                                // Release permit once we're done with the write
                                drop(permit);
                            }
                            .instrument(worker_span),
                        ))
                    }
                    WorkerRequest::Flush(callback) => {
                        // Wait on all workers to finish writing
                        while let Some(worker) = workers.next().await {
                            let _ = worker;
                        }
                        drop(callback);
                    }
                    WorkerRequest::Shutdown(callback) => {
                        shutdown_callback = Some(callback);
                        break;
                    }
                };
            }
            // Drop write consumer to immediately notify callers that cache is shutting down
            drop(write_consumer);

            // wait for all writers to finish
            while let Some(worker) = workers.next().await {
                let _ = worker;
            }
            if let Some(callback) = shutdown_callback {
                callback.send(()).ok();
            }
        });

        Ok(AsyncCache {
            real_cache,
            writer_sender,
        })
    }

    #[tracing::instrument(skip_all)]
    pub async fn put(
        &self,
        anchor: AbsoluteSystemPathBuf,
        key: String,
        files: Vec<AnchoredSystemPathBuf>,
        duration: u64,
    ) -> Result<(), CacheError> {
        if self
            .writer_sender
            .send(WorkerRequest::WriteRequest {
                anchor,
                key,
                duration,
                files,
            })
            .await
            .is_err()
        {
            Err(CacheError::CacheShuttingDown)
        } else {
            Ok(())
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn exists(&self, key: &str) -> Result<Option<CacheHitMetadata>, CacheError> {
        self.real_cache.exists(key).await
    }

    #[tracing::instrument(skip_all)]
    pub async fn fetch(
        &self,
        anchor: &AbsoluteSystemPath,
        key: &str,
    ) -> Result<Option<(CacheHitMetadata, Vec<AnchoredSystemPathBuf>)>, CacheError> {
        self.real_cache.fetch(anchor, key).await
    }

    // Used for testing to ensure that the workers resolve
    // before checking the cache.
    #[tracing::instrument(skip_all)]
    pub async fn wait(&self) -> Result<(), CacheError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.writer_sender
            .send(WorkerRequest::Flush(tx))
            .await
            .map_err(|_| CacheError::CacheShuttingDown)?;
        // Wait until flush callback is finished
        rx.await.ok();
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub async fn shutdown(&self) -> Result<(), CacheError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.writer_sender
            .send(WorkerRequest::Shutdown(tx))
            .await
            .map_err(|_| CacheError::CacheShuttingDown)?;
        rx.await.ok();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use anyhow::Result;
    use futures::future::try_join_all;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_api_client::{APIAuth, APIClient};
    use turborepo_vercel_api_mock::start_test_server;

    use crate::{
        test_cases::{get_test_cases, TestCase},
        AsyncCache, CacheHitMetadata, CacheOpts, CacheSource, RemoteCacheOpts,
    };

    #[tokio::test]
    async fn test_async_cache() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));

        try_join_all(get_test_cases().into_iter().map(|test_case| async move {
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
            remote_cache_read_only: false,
            skip_remote: false,
            skip_filesystem: true,
            workers: 10,
            remote_cache_opts: Some(RemoteCacheOpts {
                unused_team_id: Some("my-team".to_string()),
                signature: false,
            }),
        };

        let api_client = APIClient::new(format!("http://localhost:{}", port), 200, "2.0.0", true)?;
        let api_auth = Some(APIAuth {
            team_id: Some("my-team-id".to_string()),
            token: "my-token".to_string(),
            team_slug: None,
        });
        let async_cache = AsyncCache::new(&opts, &repo_root_path, api_client, api_auth, None)?;

        // Ensure that the cache is empty
        let response = async_cache.exists(&hash).await;

        assert_matches!(response, Ok(None));

        // Add test case
        async_cache
            .put(
                repo_root_path.clone(),
                hash.clone(),
                test_case
                    .files
                    .iter()
                    .map(|f| f.path().to_owned())
                    .collect(),
                test_case.duration,
            )
            .await
            .unwrap();

        // Wait for async cache to process
        async_cache.wait().await.unwrap();

        let fs_cache_path = repo_root_path.join_components(&[
            "node_modules",
            ".cache",
            "turbo",
            &format!("{}.tar.zst", hash),
        ]);

        // Confirm that fs cache file does *not* exist
        assert!(!fs_cache_path.exists());

        let response = async_cache.exists(&hash).await?;

        // Confirm that we fetch from remote cache and not local.
        assert_eq!(
            response,
            Some(CacheHitMetadata {
                source: CacheSource::Remote,
                time_saved: test_case.duration
            })
        );

        async_cache.shutdown().await.unwrap();
        assert!(
            async_cache.shutdown().await.is_err(),
            "second shutdown should error"
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
            remote_cache_read_only: false,
            skip_remote: true,
            skip_filesystem: false,
            workers: 10,
            remote_cache_opts: Some(RemoteCacheOpts {
                unused_team_id: Some("my-team".to_string()),
                signature: false,
            }),
        };

        // Initialize client with invalid API url to ensure that we don't hit the
        // network
        let api_client = APIClient::new("http://example.com", 200, "2.0.0", true)?;
        let api_auth = Some(APIAuth {
            team_id: Some("my-team-id".to_string()),
            token: "my-token".to_string(),
            team_slug: None,
        });
        let async_cache = AsyncCache::new(&opts, &repo_root_path, api_client, api_auth, None)?;

        // Ensure that the cache is empty
        let response = async_cache.exists(&hash).await;

        assert_matches!(response, Ok(None));

        // Add test case
        async_cache
            .put(
                repo_root_path.clone(),
                hash.clone(),
                test_case
                    .files
                    .iter()
                    .map(|f| f.path().to_owned())
                    .collect(),
                test_case.duration,
            )
            .await
            .unwrap();

        // Wait for async cache to process
        async_cache.wait().await.unwrap();

        let fs_cache_path = repo_root_path.join_components(&[
            "node_modules",
            ".cache",
            "turbo",
            &format!("{}.tar.zst", hash),
        ]);

        // Confirm that fs cache file exists
        assert!(fs_cache_path.exists());

        let response = async_cache.exists(&hash).await?;

        // Confirm that we fetch from local cache first.
        assert_eq!(
            response,
            Some(CacheHitMetadata {
                source: CacheSource::Local,
                time_saved: test_case.duration
            })
        );

        // Remove fs cache file
        fs_cache_path.remove_file()?;

        let response = async_cache.exists(&hash).await?;

        // Confirm that we get a cache miss
        assert!(response.is_none());

        async_cache.shutdown().await.unwrap();
        assert!(
            async_cache.shutdown().await.is_err(),
            "second shutdown should error"
        );

        Ok(())
    }

    async fn round_trip_test_with_both_caches(test_case: &TestCase, port: u16) -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        test_case.initialize(&repo_root_path)?;

        let hash = format!("{}-both", test_case.hash);

        let opts = CacheOpts {
            override_dir: None,
            remote_cache_read_only: false,
            skip_remote: false,
            skip_filesystem: false,
            workers: 10,
            remote_cache_opts: Some(RemoteCacheOpts {
                unused_team_id: Some("my-team".to_string()),
                signature: false,
            }),
        };

        let api_client = APIClient::new(format!("http://localhost:{}", port), 200, "2.0.0", true)?;
        let api_auth = Some(APIAuth {
            team_id: Some("my-team-id".to_string()),
            token: "my-token".to_string(),
            team_slug: None,
        });
        let async_cache = AsyncCache::new(&opts, &repo_root_path, api_client, api_auth, None)?;

        // Ensure that the cache is empty
        let response = async_cache.exists(&hash).await;

        assert_matches!(response, Ok(None));

        // Add test case
        async_cache
            .put(
                repo_root_path.clone(),
                hash.clone(),
                test_case
                    .files
                    .iter()
                    .map(|f| f.path().to_owned())
                    .collect(),
                test_case.duration,
            )
            .await
            .unwrap();

        // Wait for async cache to process
        async_cache.wait().await.unwrap();

        let fs_cache_path = repo_root_path.join_components(&[
            "node_modules",
            ".cache",
            "turbo",
            &format!("{}.tar.zst", hash),
        ]);

        // Confirm that fs cache file exists
        assert!(fs_cache_path.exists());

        let response = async_cache.exists(&hash).await?;

        // Confirm that we fetch from local cache first.
        assert_eq!(
            response,
            Some(CacheHitMetadata {
                source: CacheSource::Local,
                time_saved: test_case.duration
            })
        );

        // Remove fs cache file
        fs_cache_path.remove_file()?;

        let response = async_cache.exists(&hash).await?;

        // Confirm that we still can fetch from remote cache
        assert_eq!(
            response,
            Some(CacheHitMetadata {
                source: CacheSource::Remote,
                time_saved: test_case.duration
            })
        );

        async_cache.shutdown().await.unwrap();
        assert!(
            async_cache.shutdown().await.is_err(),
            "second shutdown should error"
        );

        Ok(())
    }
}
