use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::discovery::{DiscoveryResponse, Error, PackageDiscovery, WorkspaceData};

use crate::daemon::{proto::PackageManager, DaemonClient};

#[derive(Debug)]
pub struct DaemonPackageDiscovery<C> {
    daemon: DaemonClient<C>,
}

impl<C> DaemonPackageDiscovery<C> {
    pub fn new(daemon: DaemonClient<C>) -> Self {
        Self { daemon }
    }
}

impl<C: Clone + Send + Sync> PackageDiscovery for DaemonPackageDiscovery<C> {
    async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using daemon");

        // clone here so we can make concurrent requests
        let mut daemon = self.daemon.clone();

        let response = daemon
            .discover_packages()
            .await
            .map_err(|e| Error::Failed(Box::new(e)))?;

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
            package_manager: PackageManager::try_from(response.package_manager)
                .expect("valid")
                .into(),
        })
    }

    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using daemon");

        // clone here so we can make concurrent requests
        let mut daemon = self.daemon.clone();

        let response = daemon
            .discover_packages_blocking()
            .await
            .map_err(|e| Error::Failed(Box::new(e)))?;

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
            package_manager: PackageManager::try_from(response.package_manager)
                .expect("valid")
                .into(),
        })
    }
}
