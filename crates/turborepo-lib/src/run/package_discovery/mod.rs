use std::sync::Arc;

use tokio::sync::watch::Receiver;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::discovery::{DiscoveryResponse, Error, PackageDiscovery, WorkspaceData};

use crate::daemon::{proto::PackageManager, DaemonClient, FileWatching};

pub struct DaemonPackageDiscovery<'a, C: Clone> {
    daemon: &'a mut DaemonClient<C>,
}

impl<'a, C: Clone> DaemonPackageDiscovery<'a, C> {
    pub fn new(daemon: &'a mut DaemonClient<C>) -> Self {
        Self { daemon }
    }
}

impl<'a, C: Clone + Send> PackageDiscovery for DaemonPackageDiscovery<'a, C> {
    async fn discover_packages(&mut self) -> Result<DiscoveryResponse, Error> {
        let response = self
            .daemon
            .discover_packages()
            .await
            .map_err(|_| Error::Failed)?;

        Ok(DiscoveryResponse {
            workspaces: response
                .package_files
                .into_iter()
                .map(|p| WorkspaceData {
                    package_json: AbsoluteSystemPathBuf::new(p.package_json).expect("absolute"),
                    turbo_json: p
                        .turbo_json
                        .map(|t| AbsoluteSystemPathBuf::new(t).expect("absolute")),
                })
                .collect(),
            package_manager: PackageManager::from_i32(response.package_manager)
                .expect("valid")
                .into(),
        })
    }
}

/// A package discovery strategy that watches the file system for changes. Basic
/// idea:
/// - Set up a watcher on file changes on the relevant workspace file for the
///   package manager
/// - When the workspace globs change, re-discover the workspace
/// - When a package.json changes, re-discover the workspace
/// - Keep an in-memory cache of the workspace
pub struct WatchingPackageDiscovery {
    /// file watching may not be ready yet so we store a watcher
    /// through which we can get the file watching stack
    watcher: Receiver<Option<Arc<crate::daemon::FileWatching>>>,
}

impl WatchingPackageDiscovery {
    pub fn new(watcher: Receiver<Option<Arc<FileWatching>>>) -> Self {
        Self { watcher }
    }
}

impl PackageDiscovery for WatchingPackageDiscovery {
    async fn discover_packages(&mut self) -> Result<DiscoveryResponse, Error> {
        // need to clone and drop the Ref before we can await
        let watcher = {
            let watcher = self
                .watcher
                .wait_for(|opt| opt.is_some())
                .await
                .map_err(|_| Error::Failed)?;
            watcher.as_ref().expect("guaranteed some above").clone()
        };

        Ok(DiscoveryResponse {
            workspaces: watcher.package_watcher.get_package_data().await,
            package_manager: watcher.package_watcher.get_package_manager().await,
        })
    }
}

#[cfg(test)]
mod test {
    use turbopath::AbsoluteSystemPathBuf;

    use crate::daemon::DaemonConnector;

    #[tokio::test]
    async fn test_daemon_package_discovery() {
        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            pid_file: AbsoluteSystemPathBuf::new("/tmp/turbod/6c5948bd4171b931/turbod.pid")
                .unwrap(),
            sock_file: AbsoluteSystemPathBuf::new("/tmp/turbod/6c5948bd4171b931/turbod.sock")
                .unwrap(),
        };

        let mut client = connector.connect().await.unwrap();

        let packages = client.discover_packages().await.unwrap();

        println!("{:#?}", packages);
    }
}
