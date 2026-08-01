use std::{ffi::OsStr, path::Path};

use tokio::{sync::broadcast, task::JoinHandle};
use turborepo_filewatch::WatchScope;

use crate::{PackageChangeEvent, PackageChangesWatcher, PackageChangesWatcherArgs};

pub struct RediscoveringPackageChangesWatcher {
    _handle: JoinHandle<()>,
    package_changes_rx: broadcast::Receiver<PackageChangeEvent>,
}

impl RediscoveringPackageChangesWatcher {
    pub fn new(args: PackageChangesWatcherArgs) -> Self {
        let (package_changes_tx, package_changes_rx) = broadcast::channel(16);
        let _handle = tokio::spawn(async move {
            let scope = WatchScope::predicate(should_rediscover);
            let Ok(mut events) = args.file_events.subscribe(scope).await else {
                return;
            };

            loop {
                match events.recv().await {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = package_changes_tx.send(PackageChangeEvent::Rediscover);
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        Self {
            _handle,
            package_changes_rx,
        }
    }
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
