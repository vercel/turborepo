//! turborepo-discovery
//!
//! This package contains a number of strategies for discovering various things
//! about a workspace. These traits come with a basic implementation and some
//! adaptors that can be used to compose them together.
//!
//! This powers various intents such as 'query the daemon for this data, or
//! fallback to local discovery if the daemon is not available'. Eventually,
//! these strategies will implement some sort of monad-style composition so that
//! we can track areas of run that are performing sub-optimally.

use futures::StreamExt;
use tokio::time::error::Elapsed;
use tracing::Instrument;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    package_json::PackageJson,
    package_manager::{self, PackageManager},
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceData {
    workspace_root: AbsoluteSystemPathBuf,
    package_json: AbsoluteSystemPathBuf,
    turbo_json: Option<AbsoluteSystemPathBuf>,
}

impl WorkspaceData {
    pub fn new(
        package_json: AbsoluteSystemPathBuf,
        turbo_json: Option<AbsoluteSystemPathBuf>,
    ) -> Result<Self, Error> {
        let workspace_root = package_json.parent().ok_or_else(|| {
            Error::InvalidResponse("workspace package.json has no parent directory".into())
        })?;
        if turbo_json
            .as_deref()
            .is_some_and(|turbo_json| turbo_json.parent() != Some(workspace_root))
        {
            return Err(Error::InvalidResponse(
                "workspace turbo.json must be in the same directory as package.json".into(),
            ));
        }

        Ok(Self {
            workspace_root: workspace_root.to_owned(),
            package_json,
            turbo_json,
        })
    }

    pub fn workspace_root(&self) -> &AbsoluteSystemPath {
        &self.workspace_root
    }

    pub fn package_json(&self) -> &AbsoluteSystemPath {
        &self.package_json
    }

    pub fn turbo_json(&self) -> Option<&AbsoluteSystemPath> {
        self.turbo_json.as_deref()
    }

    pub fn into_paths(self) -> (AbsoluteSystemPathBuf, Option<AbsoluteSystemPathBuf>) {
        (self.package_json, self.turbo_json)
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryResponse {
    pub workspaces: Vec<WorkspaceData>,
    pub package_manager: PackageManager,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Discovery unavailable")]
    Unavailable,
    #[error("Invalid discovery response: {0}")]
    InvalidResponse(String),
    #[error("Discovery failed: {0}")]
    Failed(Box<dyn std::error::Error + Send + Sync>),
}

#[derive(thiserror::Error, Debug)]
#[error(
    "Found both turbo.json and turbo.jsonc in the same directory: {directory}\nRemove either \
     turbo.json or turbo.jsonc so there is only one."
)]
pub struct MultipleTurboConfigsError {
    pub directory: String,
}

pub fn select_turbo_config_path(
    directory: &turbopath::AbsoluteSystemPath,
    turbo_json_exists: bool,
    turbo_jsonc_exists: bool,
) -> Result<Option<AbsoluteSystemPathBuf>, MultipleTurboConfigsError> {
    match (turbo_json_exists, turbo_jsonc_exists) {
        (true, true) => Err(MultipleTurboConfigsError {
            directory: directory.to_string(),
        }),
        (true, false) => Ok(Some(directory.join_component("turbo.json"))),
        (false, true) => Ok(Some(directory.join_component("turbo.jsonc"))),
        (false, false) => Ok(None),
    }
}

pub async fn discover_turbo_config_path(
    directory: &turbopath::AbsoluteSystemPath,
) -> Result<Option<AbsoluteSystemPathBuf>, Error> {
    let turbo_json = directory.join_component("turbo.json");
    let turbo_jsonc = directory.join_component("turbo.jsonc");
    let (turbo_json_exists, turbo_jsonc_exists) = tokio::join!(
        tokio::fs::try_exists(turbo_json.as_path()),
        tokio::fs::try_exists(turbo_jsonc.as_path())
    );

    // Discovery has historically treated stat errors as a missing optional config.
    select_turbo_config_path(
        directory,
        turbo_json_exists.unwrap_or_default(),
        turbo_jsonc_exists.unwrap_or_default(),
    )
    .map_err(|error| Error::Failed(Box::new(error)))
}

/// Defines a strategy for discovering packages on the filesystem.
pub trait PackageDiscovery {
    // desugar to assert that the future is Send
    /// Discover packages on the filesystem. In the event that this would block,
    /// some strategies may return `Err(Error::Unavailable)`. If you want to
    /// wait, use `discover_packages_blocking` which will wait for the result.
    fn discover_packages(
        &self,
    ) -> impl std::future::Future<Output = Result<DiscoveryResponse, Error>> + Send;

    /// Discover packages on the filesystem, blocking until the result is ready.
    fn discover_packages_blocking(
        &self,
    ) -> impl std::future::Future<Output = Result<DiscoveryResponse, Error>> + Send;
}

/// We want to allow for lazily generating the PackageDiscovery implementation
/// to prevent unnecessary work. This trait allows us to do that.
///
/// Note: there is a blanket implementation for everything that implements
/// PackageDiscovery
pub trait PackageDiscoveryBuilder {
    type Output: PackageDiscovery;
    type Error: std::error::Error;

    fn build(self) -> Result<Self::Output, Self::Error>;
}

pub struct LocalPackageDiscovery {
    repo_root: AbsoluteSystemPathBuf,
    package_manager: PackageManager,
    discover_turbo_json: bool,
}

impl LocalPackageDiscovery {
    pub fn new(repo_root: AbsoluteSystemPathBuf, package_manager: PackageManager) -> Self {
        Self {
            repo_root,
            package_manager,
            discover_turbo_json: true,
        }
    }
}

pub struct LocalPackageDiscoveryBuilder {
    repo_root: AbsoluteSystemPathBuf,
    package_manager: Option<PackageManager>,
    package_json: Option<PackageJson>,
    allow_missing_package_manager: bool,
    discover_turbo_json: bool,
}

impl LocalPackageDiscoveryBuilder {
    pub fn new(
        repo_root: AbsoluteSystemPathBuf,
        package_manager: Option<PackageManager>,
        package_json: Option<PackageJson>,
    ) -> Self {
        Self {
            repo_root,
            package_manager,
            package_json,
            allow_missing_package_manager: false,
            discover_turbo_json: true,
        }
    }

    pub fn with_allow_no_package_manager(&mut self, allow_missing_package_manager: bool) {
        self.allow_missing_package_manager = allow_missing_package_manager;
    }

    pub fn with_package_manager(&mut self, package_manager: Option<PackageManager>) -> &mut Self {
        self.package_manager = package_manager;
        self
    }

    pub(crate) fn with_turbo_json_discovery(&mut self, discover_turbo_json: bool) -> &mut Self {
        self.discover_turbo_json = discover_turbo_json;
        self
    }
}

impl PackageDiscoveryBuilder for LocalPackageDiscoveryBuilder {
    type Output = LocalPackageDiscovery;
    type Error = package_manager::Error;

    fn build(self) -> Result<Self::Output, Self::Error> {
        let package_manager = match self.package_manager {
            Some(pm) => pm,
            None => {
                let package_json = self.package_json.map(Ok).unwrap_or_else(|| {
                    PackageJson::load(&self.repo_root.join_component("package.json"))
                })?;
                if self.allow_missing_package_manager {
                    PackageManager::get_package_manager(&self.repo_root, &package_json)
                        .or_else(|_| PackageManager::detect_package_manager(&self.repo_root))?
                } else {
                    PackageManager::get_package_manager(&self.repo_root, &package_json)?
                }
            }
        };

        Ok(LocalPackageDiscovery {
            repo_root: self.repo_root,
            package_manager,
            discover_turbo_json: self.discover_turbo_json,
        })
    }
}

impl PackageDiscovery for LocalPackageDiscovery {
    async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using local strategy");

        let glob_span = tracing::info_span!("workspace_glob_walk").entered();
        let package_paths = match self.package_manager.get_package_jsons(&self.repo_root) {
            Ok(packages) => packages,
            // if there is not a list of workspaces, it is not necessarily an error. just report no
            // workspaces
            Err(package_manager::Error::Workspace(_)) => {
                return Ok(DiscoveryResponse {
                    workspaces: vec![],
                    package_manager: self.package_manager.clone(),
                });
            }
            Err(e) => return Err(Error::Failed(Box::new(e))),
        };

        drop(glob_span);

        if !self.discover_turbo_json {
            return Ok(DiscoveryResponse {
                workspaces: package_paths
                    .into_iter()
                    .map(|package_json| WorkspaceData::new(package_json, None))
                    .collect::<Result<_, _>>()?,
                package_manager: self.package_manager.clone(),
            });
        }

        // `buffered` keeps discovery order deterministic while letting the
        // per-workspace config discovery run concurrently — sequentially these
        // per-workspace syscalls cost ~20ms on large monorepos.
        futures::stream::iter(package_paths.into_iter().map(|path| async move {
            let package_dir = path.parent().expect("non-root");
            let turbo_json = discover_turbo_config_path(package_dir).await?;

            WorkspaceData::new(path, turbo_json)
        }))
        .buffered(64)
        .collect::<Vec<Result<WorkspaceData, Error>>>()
        .instrument(tracing::info_span!("turbo_json_stat_stream"))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map(|workspaces| DiscoveryResponse {
            workspaces,
            package_manager: self.package_manager.clone(),
        })
    }

    // there is no notion of waiting for upstream deps here, so this is the same as
    // the non-blocking
    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, Error> {
        self.discover_packages().await
    }
}

/// Attempts to run the `primary` strategy for an amount of time
/// specified by `timeout` before falling back to `fallback`
pub struct FallbackPackageDiscovery<P: PackageDiscovery + Send + Sync, F> {
    primary: P,
    fallback: F,
    timeout: std::time::Duration,
}

impl<P: PackageDiscovery + Send + Sync, F: PackageDiscovery + Send + Sync>
    FallbackPackageDiscovery<P, F>
{
    pub fn new(primary: P, fallback: F, timeout: std::time::Duration) -> Self {
        Self {
            primary,
            fallback,
            timeout,
        }
    }
}

impl<T: PackageDiscovery> PackageDiscoveryBuilder for T {
    type Output = T;
    type Error = std::convert::Infallible;

    fn build(self) -> Result<Self::Output, Self::Error> {
        Ok(self)
    }
}

impl<A: PackageDiscovery + Send + Sync, B: PackageDiscovery + Send + Sync> PackageDiscovery
    for FallbackPackageDiscovery<A, B>
{
    async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using fallback strategy");

        tracing::debug!("attempting primary strategy");
        match tokio::time::timeout(self.timeout, self.primary.discover_packages()).await {
            Ok(Ok(packages)) => Ok(packages),
            Ok(Err(err1)) => {
                tracing::debug!("primary strategy failed, attempting fallback strategy");
                match self.fallback.discover_packages().await {
                    Ok(packages) => Ok(packages),
                    // if the backup is unavailable, return the original error
                    Err(Error::Unavailable) => Err(err1),
                    Err(err2) => Err(err2),
                }
            }
            Err(_) => {
                tracing::debug!("primary strategy timed out, attempting fallback strategy");
                self.fallback.discover_packages().await
            }
        }
    }

    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using fallback strategy");

        tracing::debug!("attempting primary strategy");
        match tokio::time::timeout(self.timeout, self.primary.discover_packages_blocking()).await {
            Ok(Ok(packages)) => Ok(packages),
            Ok(Err(err1)) => {
                tracing::debug!("primary strategy failed, attempting fallback strategy");
                match self.fallback.discover_packages_blocking().await {
                    Ok(packages) => Ok(packages),
                    // if the backup is unavailable, return the original error
                    Err(Error::Unavailable) => Err(err1),
                    Err(err2) => Err(err2),
                }
            }
            Err(Elapsed { .. }) => {
                tracing::debug!("primary strategy timed out, attempting fallback strategy");
                self.fallback.discover_packages_blocking().await
            }
        }
    }
}

pub struct CachingPackageDiscovery<P: PackageDiscovery> {
    primary: P,
    data: async_once_cell::OnceCell<DiscoveryResponse>,
}

impl<P: PackageDiscovery> CachingPackageDiscovery<P> {
    pub fn new(primary: P) -> Self {
        Self {
            primary,
            data: Default::default(),
        }
    }
}

impl<P: PackageDiscovery + Send + Sync> PackageDiscovery for CachingPackageDiscovery<P> {
    async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using caching strategy");
        self.data
            .get_or_try_init(async {
                tracing::debug!("discovering packages using primary strategy");
                self.primary.discover_packages().await
            })
            .await
            .map(ToOwned::to_owned)
    }

    async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using caching strategy");
        self.data
            .get_or_try_init(async {
                tracing::debug!("discovering packages using primary strategy");
                self.primary.discover_packages_blocking().await
            })
            .await
            .map(ToOwned::to_owned)
    }
}

#[cfg(test)]
mod local_tests {
    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPath;

    use super::*;

    #[test]
    fn workspace_data_owns_valid_workspace_paths() {
        let (_dir, repo_root) = npm_workspace();
        let workspace_root = repo_root.join_components(&["apps", "web"]);
        let package_json = workspace_root.join_component("package.json");
        let turbo_json = workspace_root.join_component("turbo.json");

        let workspace = WorkspaceData::new(package_json.clone(), Some(turbo_json.clone())).unwrap();
        assert_eq!(workspace.workspace_root(), &*workspace_root);
        assert_eq!(workspace.package_json(), &*package_json);
        assert_eq!(workspace.turbo_json(), Some(&*turbo_json));

        let other_turbo_json = repo_root.join_components(&["apps", "other", "turbo.json"]);
        assert!(WorkspaceData::new(package_json, Some(other_turbo_json)).is_err());

        let other_manifest = workspace_root.join_component("pyproject.toml");
        assert!(WorkspaceData::new(other_manifest, None).is_ok());
    }

    fn npm_workspace() -> (TempDir, AbsoluteSystemPathBuf) {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("apps/web")).unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces":["apps/*"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("apps/web/package.json"),
            r#"{"name":"web"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("apps/web/turbo.json"), "{}").unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(dir.path())
            .unwrap()
            .to_owned();
        (dir, repo_root)
    }

    #[tokio::test]
    async fn turbo_json_discovery_can_be_disabled() {
        let (_dir, repo_root) = npm_workspace();

        let with_turbo_json = LocalPackageDiscovery::new(repo_root.clone(), PackageManager::Npm)
            .discover_packages()
            .await
            .unwrap();
        assert_eq!(with_turbo_json.workspaces.len(), 1);
        assert!(with_turbo_json.workspaces[0].turbo_json.is_some());

        let mut builder =
            LocalPackageDiscoveryBuilder::new(repo_root, Some(PackageManager::Npm), None);
        builder.with_turbo_json_discovery(false);
        let without_turbo_json = builder.build().unwrap().discover_packages().await.unwrap();
        assert_eq!(without_turbo_json.workspaces.len(), 1);
        assert!(without_turbo_json.workspaces[0].turbo_json.is_none());
    }

    #[tokio::test]
    async fn discovers_turbo_jsonc_and_rejects_duplicate_configs() {
        let (_dir, repo_root) = npm_workspace();
        let workspace_dir = repo_root.join_components(&["apps", "web"]);
        let turbo_json = workspace_dir.join_component("turbo.json");
        let turbo_jsonc = workspace_dir.join_component("turbo.jsonc");

        turbo_json.remove_file().unwrap();
        turbo_jsonc.create_with_contents("{}").unwrap();

        let discovery = LocalPackageDiscovery::new(repo_root.clone(), PackageManager::Npm)
            .discover_packages()
            .await
            .unwrap();
        assert_eq!(discovery.workspaces[0].turbo_json, Some(turbo_jsonc));

        turbo_json.create_with_contents("{}").unwrap();

        let error = LocalPackageDiscovery::new(repo_root, PackageManager::Npm)
            .discover_packages()
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Found both turbo.json and turbo.jsonc")
        );
    }
}

#[cfg(test)]
mod fallback_tests {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use tokio::runtime::Runtime;

    use super::*;

    struct MockDiscovery {
        should_fail: bool,
        calls: AtomicUsize,
    }

    impl MockDiscovery {
        fn new(should_fail: bool) -> Self {
            Self {
                should_fail,
                calls: Default::default(),
            }
        }
    }

    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
            if self.should_fail {
                Err(Error::Failed(Box::new(std::io::Error::other("mock error"))))
            } else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                self.calls.fetch_add(1, Ordering::SeqCst);
                // Simulate successful package discovery
                Ok(DiscoveryResponse {
                    package_manager: PackageManager::Npm,
                    workspaces: vec![],
                })
            }
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            self.discover_packages().await
        }
    }

    #[test]
    fn test_fallback_on_primary_failure() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let primary = MockDiscovery::new(true);
            let fallback = MockDiscovery::new(false);

            let mut discovery =
                FallbackPackageDiscovery::new(primary, fallback, Duration::from_secs(5));

            // Invoke the method under test
            let result = discovery.discover_packages().await;

            // Assert that the fallback was used and successful
            assert!(result.is_ok());

            // Assert that the fallback was used
            assert_eq!(*discovery.primary.calls.get_mut(), 0);
            assert_eq!(*discovery.fallback.calls.get_mut(), 1);
        });
    }

    #[test]
    fn test_fallback_on_primary_timeout() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let primary = MockDiscovery::new(false);
            let fallback = MockDiscovery::new(false);

            let mut discovery =
                FallbackPackageDiscovery::new(primary, fallback, Duration::from_millis(1));

            // Invoke the method under test
            let result = discovery.discover_packages().await;

            // Assert that the fallback was used and successful
            assert!(result.is_ok());

            // Assert that the fallback was used
            assert_eq!(*discovery.primary.calls.get_mut(), 0);
            assert_eq!(*discovery.fallback.calls.get_mut(), 1);
        });
    }
}

#[cfg(test)]
mod caching_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::runtime::Runtime;

    use super::*;

    struct MockPackageDiscovery {
        call_count: AtomicUsize,
    }

    impl PackageDiscovery for MockPackageDiscovery {
        async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            // Simulate successful package discovery
            Ok(DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            self.discover_packages().await
        }
    }

    #[test]
    fn test_caching_package_discovery() {
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            let primary = MockPackageDiscovery {
                call_count: Default::default(),
            };
            let mut discovery = CachingPackageDiscovery::new(primary);

            // First call should use primary discovery
            let _first_result = discovery.discover_packages().await.unwrap();
            assert_eq!(*discovery.primary.call_count.get_mut(), 1);

            // Second call should use cached data and not increase call count
            let _second_result = discovery.discover_packages().await.unwrap();
            assert_eq!(*discovery.primary.call_count.get_mut(), 1);
        });
    }
}
