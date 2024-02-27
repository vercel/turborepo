use notify::Event;
use tokio::sync::{broadcast, oneshot, watch};
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{ChangeMapper, PackageChanges},
    package_graph::{PackageGraphBuilder, PackageName},
    package_json::PackageJson,
};

use crate::{NotifyError, OptionalWatch};

pub enum PackageChangeEvent {
    // We might want to make this just String
    Package { name: PackageName },
    Rediscover,
}

/// Watches for changes to a package's files and directories.
pub struct PackageChangesWatcher {
    _exit_tx: oneshot::Sender<()>,
    _handle: tokio::task::JoinHandle<()>,
    package_change_events_rx: watch::Receiver<PackageChangeEvent>,
}

impl PackageChangesWatcher {
    pub fn new(
        repo_root: AbsoluteSystemPathBuf,
        file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    ) -> Self {
        let (exit_tx, exit_rx) = oneshot::channel();
        let (package_change_events_tx, package_change_events_rx) =
            watch::channel(PackageChangeEvent::Rediscover);
        let subscriber = Subscriber::new(repo_root, file_events_lazy, package_change_events_tx);

        let _handle = tokio::spawn(subscriber.watch(exit_rx));
        Self {
            _exit_tx: exit_tx,
            _handle,
            package_change_events_rx,
        }
    }

    pub fn package_changes(&self) -> watch::Receiver<PackageChangeEvent> {
        self.package_change_events_rx.clone()
    }
}

struct Subscriber {
    file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
    repo_root: AbsoluteSystemPathBuf,
    package_change_events_tx: watch::Sender<PackageChangeEvent>,
}

impl Subscriber {
    fn new(
        repo_root: AbsoluteSystemPathBuf,
        file_events_lazy: OptionalWatch<broadcast::Receiver<Result<Event, NotifyError>>>,
        package_change_events_tx: watch::Sender<PackageChangeEvent>,
    ) -> Self {
        Subscriber {
            repo_root,
            file_events_lazy,
            package_change_events_tx,
        }
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

            let Ok(root_package_json) =
                PackageJson::load(&self.repo_root.join_component("package.json"))
            else {
                tracing::debug!("no package.json found, package watcher not available");
                return;
            };
            let Ok(pkg_dep_graph) = PackageGraphBuilder::new(&self.repo_root, root_package_json)
                .build()
                .await
            else {
                tracing::debug!("package graph not available, package watcher not available");
                return;
            };

            // TODO: Pass in global_deps and ignore_patterns
            let change_mapper = ChangeMapper::new(&pkg_dep_graph, vec![], vec![]);

            loop {
                match file_events.recv().await {
                    Ok(Ok(Event { paths, .. })) => {
                        let changed_files = paths
                            .into_iter()
                            .filter_map(|p| {
                                let p = AbsoluteSystemPathBuf::try_from(p).ok()?;
                                self.repo_root.anchor(&p).ok()
                            })
                            .collect();

                        let changes = change_mapper.changed_packages(changed_files, None);
                        match changes {
                            Ok(PackageChanges::All) => {
                                let _ = self
                                    .package_change_events_tx
                                    .send(PackageChangeEvent::Rediscover);
                            }
                            Ok(PackageChanges::Some(changed_pkgs)) => {
                                for pkg in changed_pkgs {
                                    let _ = self.package_change_events_tx.send(
                                        PackageChangeEvent::Package {
                                            name: pkg.name.clone(),
                                        },
                                    );
                                }
                            }
                            Err(err) => {
                                tracing::error!("PACKAGE WATCH error: {:?}", err);
                            }
                        }
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
