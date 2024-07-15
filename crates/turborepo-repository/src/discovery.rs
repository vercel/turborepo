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

use tokio::time::error::Elapsed;
use tokio_stream::{iter, StreamExt};
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    package_json::PackageJson,
    package_manager::{self, PackageManager},
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceData {
    pub package_json: AbsoluteSystemPathBuf,
    pub turbo_json: Option<AbsoluteSystemPathBuf>,
}

#[derive(Debug, Clone)]
pub struct DiscoveryResponse {
    pub workspaces: Vec<WorkspaceData>,
    pub package_manager: PackageManager,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("discovery unavailable")]
    Unavailable,
    #[error("discovery failed: {0}")]
    Failed(Box<dyn std::error::Error + Send + Sync>),
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
}

impl LocalPackageDiscovery {
    pub fn new(repo_root: AbsoluteSystemPathBuf, package_manager: PackageManager) -> Self {
        Self {
            repo_root,
            package_manager,
        }
    }
}

pub struct LocalPackageDiscoveryBuilder {
    repo_root: AbsoluteSystemPathBuf,
    package_manager: Option<PackageManager>,
    package_json: Option<PackageJson>,
    allow_missing_package_manager: bool,
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
        }
    }

    pub fn with_allow_no_package_manager(&mut self, allow_missing_package_manager: bool) {
        self.allow_missing_package_manager = allow_missing_package_manager;
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
                    PackageManager::read_or_detect_package_manager(&package_json, &self.repo_root)?
                } else {
                    PackageManager::get_package_manager(&package_json)?
                }
            }
        };

        Ok(LocalPackageDiscovery {
            repo_root: self.repo_root,
            package_manager,
        })
    }
}

impl PackageDiscovery for LocalPackageDiscovery {
    async fn discover_packages(&self) -> Result<DiscoveryResponse, Error> {
        tracing::debug!("discovering packages using local strategy");

        let package_paths = match self.package_manager.get_package_jsons(&self.repo_root) {
            Ok(packages) => packages,
            // if there is not a list of workspaces, it is not necessarily an error. just report no
            // workspaces
            Err(package_manager::Error::Workspace(_)) => {
                return Ok(DiscoveryResponse {
                    workspaces: vec![],
                    package_manager: self.package_manager,
                })
            }
            Err(e) => return Err(Error::Failed(Box::new(e))),
        };

        iter(package_paths)
            .then(|path| async move {
                let potential_turbo = path
                    .parent()
                    .expect("non-root")
                    .join_component("turbo.json");
                let potential_turbo_exists = tokio::fs::try_exists(potential_turbo.as_path()).await;

                Ok(WorkspaceData {
                    package_json: path,
                    turbo_json: potential_turbo_exists
                        .unwrap_or_default()
                        .then_some(potential_turbo),
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .await
            .map(|workspaces| DiscoveryResponse {
                workspaces,
                package_manager: self.package_manager,
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
                Err(Error::Failed(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "mock error",
                ))))
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
