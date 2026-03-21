use turbopath::AbsoluteSystemPathBuf;
use turborepo_daemon::{proto::PackageManager, DaemonClient};
use turborepo_repository::{
    discovery::{DiscoveryResponse, Error, PackageDiscovery, WorkspaceData},
    workspace_provider::WorkspaceProviderId,
};

fn workspace_data_from_daemon(package: turborepo_daemon::proto::PackageFiles) -> WorkspaceData {
    let provider_id = package
        .provider_id
        .as_deref()
        .unwrap_or("node")
        .parse()
        .unwrap_or(WorkspaceProviderId::Node);
    let manifest_path = package
        .manifest_path
        .unwrap_or_else(|| package.package_json.clone());
    let package_json = if package.package_json.is_empty() {
        manifest_path.clone()
    } else {
        package.package_json
    };

    WorkspaceData {
        provider_id,
        manifest_path: AbsoluteSystemPathBuf::new(manifest_path).expect("absolute"),
        package_json: AbsoluteSystemPathBuf::new(package_json).expect("absolute"),
        turbo_json: package
            .turbo_json
            .map(|t| AbsoluteSystemPathBuf::new(t).expect("absolute")),
    }
}

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
                .map(workspace_data_from_daemon)
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
                .map(workspace_data_from_daemon)
                .collect(),
            package_manager: PackageManager::try_from(response.package_manager)
                .expect("valid")
                .into(),
        })
    }
}
