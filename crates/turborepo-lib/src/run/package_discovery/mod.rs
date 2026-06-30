use turbopath::AbsoluteSystemPathBuf;
use turborepo_daemon::{
    proto::{DiscoverPackagesResponse, PackageFiles, PackageManager as ProtoPackageManager},
    DaemonClient,
};
use turborepo_repository::{
    discovery::{DiscoveryResponse, Error, PackageDiscovery, WorkspaceData},
    package_manager::PackageManager,
};

#[derive(Debug)]
pub struct DaemonPackageDiscovery<C> {
    daemon: DaemonClient<C>,
    repo_root: AbsoluteSystemPathBuf,
}

impl<C> DaemonPackageDiscovery<C> {
    pub fn new(daemon: DaemonClient<C>, repo_root: AbsoluteSystemPathBuf) -> Self {
        Self { daemon, repo_root }
    }
}

fn workspace_data_from_proto(package_files: PackageFiles) -> Result<WorkspaceData, Error> {
    let package_json = AbsoluteSystemPathBuf::new(package_files.package_json).map_err(|err| {
        Error::InvalidResponse(format!("daemon returned invalid package.json path: {err}"))
    })?;
    let turbo_json = package_files
        .turbo_json
        .map(|path| {
            AbsoluteSystemPathBuf::new(path).map_err(|err| {
                Error::InvalidResponse(format!("daemon returned invalid turbo.json path: {err}"))
            })
        })
        .transpose()?;

    Ok(WorkspaceData {
        package_json,
        turbo_json,
    })
}

fn discovery_response_from_proto(
    response: DiscoverPackagesResponse,
    repo_root: &turbopath::AbsoluteSystemPath,
) -> Result<DiscoveryResponse, Error> {
    let package_manager: PackageManager = ProtoPackageManager::try_from(response.package_manager)
        .map_err(|_| {
            Error::InvalidResponse(format!(
                "daemon returned invalid package manager: {}",
                response.package_manager
            ))
        })?
        .into();
    let workspaces = response
        .package_files
        .into_iter()
        .map(workspace_data_from_proto)
        .collect::<Result<_, _>>()?;

    Ok(DiscoveryResponse {
        workspaces,
        package_manager: package_manager.with_resolved_nub_lockfile(repo_root),
    })
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

        discovery_response_from_proto(response, &self.repo_root)
    }

    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using daemon");

        // clone here so we can make concurrent requests
        let mut daemon = self.daemon.clone();

        let response = daemon
            .discover_packages_blocking()
            .await
            .map_err(|e| Error::Failed(Box::new(e)))?;

        discovery_response_from_proto(response, &self.repo_root)
    }
}
