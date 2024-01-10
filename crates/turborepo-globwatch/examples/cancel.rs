use std::{path::PathBuf, time::Duration};

use futures::{join, StreamExt};
use globwatch::GlobWatcher;
use tracing::{info, info_span};
use turbopath::AbsoluteSystemPathBuf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let flush_dir = AbsoluteSystemPathBuf::from_cwd("./flush").unwrap();
    let (watcher, config) = GlobWatcher::new(&flush_dir).unwrap();
    let stop = stop_token::StopSource::new();
    let mut stream = watcher.into_stream(stop.token());

    let watch_fut = async {
        let span = info_span!("watch_fut");
        let _ = span.enter();
        while let Some(Ok(e)) = stream.next().await {
            info!(parent: &span, "{:?}", e);
        }
        info!(parent: &span, "done");
    };

    let config_fut = async {
        let span = info_span!("config_fut");
        let _ = span.enter();
        for x in 0..5 {
            info!(parent: &span, "iteration {}", x);
            config
                .include(&PathBuf::from("."), "globwatch/src/**")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
            config
                .exclude(&PathBuf::from("."), "globwatch/src/**")
                .await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        info!(parent: &span, "dropping stop");
        drop(stop);
    };

    join!(watch_fut, config_fut);
}
