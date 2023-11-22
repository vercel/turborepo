mod bump_timeout;
mod bump_timeout_layer;
mod client;
mod connector;
pub(crate) mod endpoint;
mod server;

pub use client::{DaemonClient, DaemonError};
pub use connector::{DaemonConnector, DaemonConnectorError};
pub use server::{CloseReason, FileWatching, TurboGrpcService};

pub(crate) mod proto {

    tonic::include_proto!("turbodprotocol");
    /// The version of the protocol that this library implements.
    ///
    /// Protocol buffers aim to be backward and forward compatible at a protocol
    /// level, however that doesn't mean that our daemon will have the same
    /// logical API. We may decide to change the API in the future, and this
    /// version number will be used to indicate that.
    ///
    /// Changes are driven by the server changing its implementation.
    ///
    /// Guideline for bumping the daemon protocol version:
    /// - Bump the major version if making backwards incompatible changes.
    /// - Bump the minor version if adding new features, such that clients can
    ///   mandate at least some set of features on the target server.
    /// - Bump the patch version if making backwards compatible bug fixes.
    pub const VERSION: &str = "1.11.0";

    impl From<PackageManager> for turborepo_repository::package_manager::PackageManager {
        fn from(pm: PackageManager) -> Self {
            match pm {
                PackageManager::Npm => Self::Npm,
                PackageManager::Yarn => Self::Yarn,
                PackageManager::Berry => Self::Berry,
                PackageManager::Pnpm => Self::Pnpm,
                PackageManager::Pnpm6 => Self::Pnpm6,
                PackageManager::Bun => Self::Bun,
            }
        }
    }

    impl From<turborepo_repository::package_manager::PackageManager> for PackageManager {
        fn from(pm: turborepo_repository::package_manager::PackageManager) -> Self {
            match pm {
                turborepo_repository::package_manager::PackageManager::Npm => Self::Npm,
                turborepo_repository::package_manager::PackageManager::Yarn => Self::Yarn,
                turborepo_repository::package_manager::PackageManager::Berry => Self::Berry,
                turborepo_repository::package_manager::PackageManager::Pnpm => Self::Pnpm,
                turborepo_repository::package_manager::PackageManager::Pnpm6 => Self::Pnpm6,
                turborepo_repository::package_manager::PackageManager::Bun => Self::Bun,
            }
        }
    }
}
