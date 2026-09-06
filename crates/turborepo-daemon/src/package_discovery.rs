use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{
    discovery::{DiscoveryResponse, Error, PackageDiscovery, WorkspaceData},
    package_graph::PackageName,
    package_json::PackageJson,
    package_manager::PackageManager,
    toolchain::ToolchainId,
};

use crate::{
    proto::{DiscoverPackagesResponse, RepositoryScope},
    DaemonClient,
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

fn javascript_workspace_from_proto(scope: RepositoryScope) -> Result<Option<WorkspaceData>, Error> {
    if scope.toolchain != ToolchainId::JAVASCRIPT.as_str()
        || scope.name == PackageName::Root.as_str()
    {
        return Ok(None);
    }

    let package_json = AbsoluteSystemPathBuf::new(scope.manifest_path).map_err(|err| {
        Error::InvalidResponse(format!(
            "daemon returned invalid JavaScript manifest path: {err}"
        ))
    })?;

    WorkspaceData::new(package_json, None).map(Some)
}

fn discovery_response_from_proto(
    response: DiscoverPackagesResponse,
    repo_root: &turbopath::AbsoluteSystemPath,
) -> Result<DiscoveryResponse, Error> {
    let root_package_json = PackageJson::load(&repo_root.join_component("package.json"))
        .map_err(|error| Error::Failed(Box::new(error)))?;
    let package_manager =
        PackageManager::read_or_detect_package_manager(&root_package_json, repo_root)
            .map_err(|error| Error::Failed(Box::new(error)))?;
    let workspaces = response
        .scopes
        .into_iter()
        .map(javascript_workspace_from_proto)
        .filter_map(Result::transpose)
        .collect::<Result<_, _>>()?;

    Ok(DiscoveryResponse {
        workspaces,
        package_manager: package_manager.with_resolved_nub_lockfile(repo_root),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_adapter_filters_generic_repository_scopes() {
        let javascript = RepositoryScope {
            name: "web".to_string(),
            toolchain: ToolchainId::JAVASCRIPT.as_str().to_string(),
            manifest_path: if cfg!(windows) {
                r"C:\repo\apps\web\package.json".to_string()
            } else {
                "/repo/apps/web/package.json".to_string()
            },
        };
        let rust = RepositoryScope {
            name: "api".to_string(),
            toolchain: ToolchainId::RUST.as_str().to_string(),
            manifest_path: if cfg!(windows) {
                r"C:\repo\crates\api\Cargo.toml".to_string()
            } else {
                "/repo/crates/api/Cargo.toml".to_string()
            },
        };

        assert!(javascript_workspace_from_proto(javascript)
            .unwrap()
            .is_some());
        assert!(javascript_workspace_from_proto(rust).unwrap().is_none());
    }
}

impl<C: Clone + Send + Sync> PackageDiscovery for DaemonPackageDiscovery<C> {
    async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using daemon");

        let mut daemon = self.daemon.clone();
        let response = daemon
            .discover_packages()
            .await
            .map_err(|e| Error::Failed(Box::new(e)))?;

        discovery_response_from_proto(response, &self.repo_root)
    }

    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using daemon");

        let mut daemon = self.daemon.clone();
        let response = daemon
            .discover_packages_blocking()
            .await
            .map_err(|e| Error::Failed(Box::new(e)))?;

        discovery_response_from_proto(response, &self.repo_root)
    }
}
