mod bump_timeout;
mod bump_timeout_layer;
mod client;
mod connector;
pub(crate) mod endpoint;
mod server;

pub use client::{DaemonClient, DaemonError};
pub use connector::{DaemonConnector, DaemonConnectorError};
pub use server::{CloseReason, TurboGrpcService};
use sha2::{Digest, Sha256};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub pid_file: AbsoluteSystemPathBuf,
    pub lock_file: AbsoluteSystemPathBuf,
    pub sock_file: AbsoluteSystemPathBuf,
    pub lsp_pid_file: AbsoluteSystemPathBuf,
    pub log_file: AbsoluteSystemPathBuf,
    pub log_folder: AbsoluteSystemPathBuf,
}

fn repo_hash(repo_root: &AbsoluteSystemPath) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.to_string().as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

fn daemon_file_root(repo_hash: &str) -> AbsoluteSystemPathBuf {
    AbsoluteSystemPathBuf::new(std::env::temp_dir().to_str().expect("UTF-8 path"))
        .expect("temp dir is valid")
        .join_component("turbod")
        .join_component(repo_hash)
}

fn daemon_log_file_and_folder(repo_hash: &str) -> (AbsoluteSystemPathBuf, AbsoluteSystemPathBuf) {
    let directories = directories::ProjectDirs::from("com", "turborepo", "turborepo")
        .expect("user has a home dir");

    let folder = AbsoluteSystemPathBuf::new(directories.data_dir().to_str().expect("UTF-8 path"))
        .expect("absolute");

    let log_folder = folder.join_component("logs");
    let log_file = log_folder.join_component(format!("{}-turbo.log", repo_hash).as_str());

    (log_file, log_folder)
}

impl Paths {
    pub fn from_repo_root(repo_root: &AbsoluteSystemPath) -> Self {
        let repo_hash = repo_hash(repo_root);
        let daemon_root = daemon_file_root(&repo_hash);
        let (log_file, log_folder) = daemon_log_file_and_folder(&repo_hash);
        Self {
            pid_file: daemon_root.join_component("turbod.pid"),
            lock_file: daemon_root.join_component("turbod.lock"),
            sock_file: daemon_root.join_component("turbod.sock"),
            lsp_pid_file: daemon_root.join_component("lsp.pid"),
            log_file,
            log_folder,
        }
    }
}

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

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use super::repo_hash;

    #[cfg(not(target_os = "windows"))]
    #[test_case("/tmp/turborepo", "6e0cfa616f75a61c"; "basic example")]
    fn test_repo_hash(path: &str, expected_hash: &str) {
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let hash = repo_hash(&repo_root);

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }

    #[cfg(target_os = "windows")]
    #[test_case("C:\\\\tmp\\turborepo", "0103736e6883e35f"; "basic example")]
    fn test_repo_hash_win(path: &str, expected_hash: &str) {
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let hash = repo_hash(&repo_root);

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }
}
