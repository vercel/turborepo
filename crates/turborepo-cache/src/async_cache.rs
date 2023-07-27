use std::future::Future;

use futures::{stream::FuturesUnordered, StreamExt};
use tokio::task::JoinHandle;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

use crate::multiplexer::CacheMultiplexer;

struct AsyncCache {
    workers: FuturesUnordered<JoinHandle<()>>,
    max_workers: usize,
    real_cache: CacheMultiplexer,
}

impl AsyncCache {
    pub async fn put(
        &mut self,
        anchor: &AbsoluteSystemPath,
        key: &str,
        files: Vec<AnchoredSystemPathBuf>,
        duration: u32,
        token: &str,
    ) {
        if self.workers.len() >= self.max_workers {
            self.workers.next().await.unwrap();
        }

        let fut = tokio::spawn(async move {
            let _ = self
                .real_cache
                .put(&anchor, &key, files, duration, token)
                .await;
        });
        self.workers.push(fut);
    }

    pub fn new(real_cache: CacheMultiplexer, max_workers: usize) -> AsyncCache {
        AsyncCache {
            workers: FuturesUnordered::new(),
            real_cache,
            max_workers,
        }
    }
}
