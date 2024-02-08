use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::discovery::{DiscoveryResponse, Error, PackageDiscovery, WorkspaceData};

use crate::daemon::{proto::PackageManager, DaemonClient};

#[derive(Debug)]
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
        tracing::debug!("discovering packages using daemon");

        let response = self
            .daemon
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
            package_manager: PackageManager::from_i32(response.package_manager)
                .expect("valid")
                .into(),
        })
    }
}
