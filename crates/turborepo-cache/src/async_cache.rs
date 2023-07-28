use std::sync::Arc;

use futures::{stream::FuturesUnordered, StreamExt};
use tokio::task::JoinHandle;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_api_client::APIClient;

use crate::{multiplexer::CacheMultiplexer, CacheError, CacheOpts, CacheResponse};

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
        team_id: &str,
        token: &str,
    ) -> Result<AsyncCache, CacheError> {
        let max_workers = opts.workers.try_into().expect("usize is smaller than u32");
        let real_cache = CacheMultiplexer::new(opts, repo_root, api_client, team_id, token)?;

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
        &mut self,
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

    pub async fn shutdown(self) {
        for worker in self.workers {
            let _ = worker.await;
        }
    }
}
