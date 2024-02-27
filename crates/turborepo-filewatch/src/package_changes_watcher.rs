use notify::Event;
use tokio::sync::{broadcast, oneshot};

use crate::{NotifyError, OptionalWatch};

/// Watches for changes to a package's files and directories.
pub struct PackageChangesWatcher {
    _exit_tx: oneshot::Sender<()>,
    _handle: tokio::task::JoinHandle<()>,
}

impl PackageChangesWatcher {
    pub fn new(
        file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    ) -> Self {
        let (exit_tx, exit_rx) = oneshot::channel();
        let subscriber = Subscriber::new(file_events_lazy);

        let _handle = tokio::spawn(subscriber.watch(exit_rx));
        Self {
            _exit_tx: exit_tx,
            _handle,
        }
    }
}

struct Subscriber {
    file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
}

impl Subscriber {
    fn new(
        file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    ) -> Self {
        Subscriber { file_events_lazy }
    }

    async fn watch(mut self, exit_rx: oneshot::Receiver<()>) {
        let process = async {
            let Ok(mut file_events) = self.file_events_lazy.get().await.map(|r| r.resubscribe())
            else {
                // if we get here, it means that file watching has not started, so we should
                // just report that the package watcher is not available
                tracing::debug!("file watching shut down, package watcher not available");
                return;
            };

            loop {
                match file_events.recv().await {
                    Ok(Ok(event)) => {
                        tracing::debug!("PACKAGE WATCH file event: {:?}", event);
                    }
                    Ok(Err(err)) => {
                        tracing::error!("PACKAGE WATCH file event error: {:?}", err);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        tracing::warn!("PACKAGE WATCH file event lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!("PACKAGE WATCH file event channel closed");
                        break;
                    }
                }
            }
        };

        tokio::select! {
            biased;
            _ = exit_rx => {
                tracing::debug!("exiting package changes watcher due to signal");
            },
            _ = process => {
                tracing::debug!("exiting package changes watcher due to process end");
            }
        }
    }
}
