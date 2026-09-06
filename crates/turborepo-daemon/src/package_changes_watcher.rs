use std::{ffi::OsStr, path::Path, sync::Arc};

use tokio::{
    sync::{broadcast, watch},
    task::JoinHandle,
};
use turborepo_filewatch::WatchScope;
use turborepo_repository::{
    package_graph::{PackageGraph, RepositoryDiscoverySnapshot},
    package_json::PackageJson,
};

use crate::{
    PackageChangeEvent, PackageChangesWatcher, PackageChangesWatcherArgs, RepositoryDiscoveryError,
};

pub struct RediscoveringPackageChangesWatcher {
    _handle: JoinHandle<()>,
    package_changes_rx: broadcast::Receiver<PackageChangeEvent>,
    repository_discovery_rx: watch::Receiver<Option<Arc<RepositoryDiscoverySnapshot>>>,
}

impl RediscoveringPackageChangesWatcher {
    pub fn new(args: PackageChangesWatcherArgs) -> Self {
        let (package_changes_tx, package_changes_rx) = broadcast::channel(16);
        let (repository_discovery_tx, repository_discovery_rx) = watch::channel(None);
        let _handle = tokio::spawn(async move {
            if let Some(snapshot) = javascript_discovery_snapshot(&args).await {
                repository_discovery_tx.send_replace(Some(Arc::new(snapshot)));
            }

            let scope = WatchScope::predicate(should_rediscover);
            let Ok(mut events) = args.file_events.subscribe(scope).await else {
                return;
            };

            loop {
                match events.recv().await {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(snapshot) = javascript_discovery_snapshot(&args).await {
                            repository_discovery_tx.send_replace(Some(Arc::new(snapshot)));
                        }
                        let _ = package_changes_tx.send(PackageChangeEvent::Rediscover);
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        Self {
            _handle,
            package_changes_rx,
            repository_discovery_rx,
        }
    }
}

async fn javascript_discovery_snapshot(
    args: &PackageChangesWatcherArgs,
) -> Option<RepositoryDiscoverySnapshot> {
    let root_package_json =
        PackageJson::load(&args.repo_root.join_component("package.json")).ok()?;
    PackageGraph::builder(&args.repo_root, root_package_json)
        .with_allow_no_package_manager(args.allow_no_package_manager)
        .build()
        .await
        .ok()
        .map(|graph| graph.repository_discovery_snapshot())
}

fn should_rediscover(path: &Path) -> bool {
    !path.components().any(|component| {
        component.as_os_str() == OsStr::new(".git") || component.as_os_str() == OsStr::new(".turbo")
    })
}

impl PackageChangesWatcher for RediscoveringPackageChangesWatcher {
    async fn package_changes(&self) -> broadcast::Receiver<PackageChangeEvent> {
        self.package_changes_rx.resubscribe()
    }

    async fn repository_discovery(
        &self,
    ) -> Option<Result<Arc<RepositoryDiscoverySnapshot>, RepositoryDiscoveryError>> {
        self.repository_discovery_rx.borrow().clone().map(Ok)
    }

    async fn repository_discovery_blocking(
        &self,
    ) -> Result<Arc<RepositoryDiscoverySnapshot>, RepositoryDiscoveryError> {
        let mut receiver = self.repository_discovery_rx.clone();
        loop {
            if let Some(snapshot) = receiver.borrow_and_update().clone() {
                return Ok(snapshot);
            }
            receiver
                .changed()
                .await
                .map_err(|_| RepositoryDiscoveryError::Unavailable)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn ignores_daemon_and_git_files() {
        assert!(!super::should_rediscover(Path::new(
            "repo/.turbo/cookies/1"
        )));
        assert!(!super::should_rediscover(Path::new("repo/.git/index")));
        assert!(super::should_rediscover(Path::new(
            "repo/apps/web/package.json"
        )));
    }
}
