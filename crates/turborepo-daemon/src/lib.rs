//! The Turborepo daemon watches files and pre-computes data to speed up turbo's
//! execution. Each repository has a separate daemon instance.
//!
//! # Architecture
//! The daemon consists of a gRPC server that can be queried by a client.

//! The server spins up a `FileWatching` struct, which contains a struct
//! responsible for watching the repository (`FileSystemWatcher`), and the
//! various consumers of that file change data such as `GlobWatcher` and
//! `PackageWatcher`.
//!
//! We use cookie files to ensure proper event synchronization, i.e.
//! that we don't get stale file system events while handling queries.
//!
//! # Naming Conventions
//! `recv` is a receiver of file system events. Structs such as `GlobWatcher`
//! or `PackageWatcher` consume these file system events and either derive state
//! or produce new events.
//!
//! `_tx`/`_rx` suffixes indicate that this variable is respectively a `Sender`
//! or `Receiver`.

#![allow(unused_features, reason = "impl_trait_in_assoc_type is actually used")]
#![feature(impl_trait_in_assoc_type)]
#![deny(clippy::all)]
#![allow(clippy::needless_lifetimes)]
#![allow(clippy::uninlined_format_args)]

mod bump_timeout;
mod bump_timeout_layer;
mod client;
mod connector;
mod default_timeout_layer;
pub mod endpoint;
mod server;

use std::{collections::HashSet, path::PathBuf, sync::Arc};

pub use client::{DaemonClient, DaemonError};
pub use connector::{DaemonConnector, DaemonConnectorError};
pub use server::{CloseReason, FileWatching, TurboGrpcService};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, PathError};
use turborepo_repository::package_graph::PackageName;

/// Trait for watching package changes. Implemented by consumers who need
/// to integrate with turbo.json configuration.
pub trait PackageChangesWatcher: Send + Sync {
    /// Get a receiver for package change events
    fn package_changes(
        &self,
    ) -> impl std::future::Future<Output = broadcast::Receiver<PackageChangeEvent>> + Send;
}

/// Arguments passed to a PackageChangesWatcher factory
pub struct PackageChangesWatcherArgs {
    pub repo_root: AbsoluteSystemPathBuf,
    pub file_events: turborepo_filewatch::OptionalWatch<
        broadcast::Receiver<Result<notify::Event, turborepo_filewatch::NotifyError>>,
    >,
    pub hash_watcher: std::sync::Arc<turborepo_filewatch::hash_watcher::HashWatcher>,
    pub custom_turbo_json_path: Option<AbsoluteSystemPathBuf>,
    pub allow_no_package_manager: bool,
}

/// Events that indicate package changes in the repository.
#[derive(Clone, Debug)]
pub enum PackageChangeEvent {
    /// A specific package has changed.
    ///
    /// `changed_files` contains the repo-relative paths that triggered this
    /// event. Shared via `Arc` so broadcast clones are cheap. When file-level
    /// information is unavailable (e.g. daemon gRPC), this will be an empty
    /// set.
    Package {
        name: PackageName,
        changed_files: Arc<HashSet<AnchoredSystemPathBuf>>,
    },
    /// All packages need to be rediscovered
    Rediscover,
}

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
    hasher.update(repo_hash_input(repo_root).as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

#[cfg(windows)]
fn repo_hash_input(repo_root: &AbsoluteSystemPath) -> String {
    let path = repo_root.to_string();
    let mut chars = path.chars();

    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            format!("{}:{}", drive.to_ascii_uppercase(), chars.as_str())
        }
        _ => path,
    }
}

#[cfg(not(windows))]
fn repo_hash_input(repo_root: &AbsoluteSystemPath) -> String {
    repo_root.to_string()
}

#[cfg(unix)]
fn daemon_file_root(repo_hash: &str) -> Result<AbsoluteSystemPathBuf, PathError> {
    daemon_file_root_from_temp_dir(repo_hash, std::env::temp_dir())
}

#[cfg(unix)]
fn daemon_file_root_from_temp_dir(
    repo_hash: &str,
    temp_dir: PathBuf,
) -> Result<AbsoluteSystemPathBuf, PathError> {
    let uid = unsafe { libc::geteuid() };
    Ok(AbsoluteSystemPathBuf::try_from(temp_dir)?
        .join_component(format!("turbod-{uid}").as_str())
        .join_component(repo_hash))
}

#[cfg(windows)]
fn daemon_file_root(repo_hash: &str) -> Result<AbsoluteSystemPathBuf, PathError> {
    let root = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    Ok(AbsoluteSystemPathBuf::try_from(root)?
        .join_component("turbod")
        .join_component(repo_hash))
}

#[cfg(not(any(unix, windows)))]
fn daemon_file_root(repo_hash: &str) -> Result<AbsoluteSystemPathBuf, PathError> {
    Ok(AbsoluteSystemPathBuf::try_from(std::env::temp_dir())?
        .join_component("turbod")
        .join_component(repo_hash))
}

fn daemon_log_file_and_folder(
    repo_root: &AbsoluteSystemPath,
    repo_hash: &str,
) -> (AbsoluteSystemPathBuf, AbsoluteSystemPathBuf) {
    let log_folder = repo_root.join_components(&[".turbo", "daemon"]);
    let log_file = log_folder.join_component(format!("{repo_hash}-turbo.log").as_str());

    (log_file, log_folder)
}

impl Paths {
    pub fn from_repo_root(repo_root: &AbsoluteSystemPath) -> Result<Self, PathError> {
        let repo_hash = repo_hash(repo_root);
        let daemon_root = daemon_file_root(&repo_hash)?;
        let (log_file, log_folder) = daemon_log_file_and_folder(repo_root, &repo_hash);
        Ok(Self {
            pid_file: daemon_root.join_component("turbod.pid"),
            lock_file: daemon_root.join_component("turbod.lock"),
            sock_file: daemon_root.join_component("turbod.sock"),
            lsp_pid_file: daemon_root.join_component("lsp.pid"),
            log_file,
            log_folder,
        })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn daemon_file_root_rejects_non_utf8_temp_dir_without_panicking() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

        use turbopath::PathError;

        let temp_dir = PathBuf::from(OsString::from_vec(b"/tmp/turbo-\xFF".to_vec()));

        assert!(matches!(
            super::daemon_file_root_from_temp_dir("repo-hash", temp_dir),
            Err(PathError::FromPathBufError(_))
        ));
    }
}

pub mod proto {

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
    pub const VERSION: &str = "2.0.0";

    impl From<PackageManager> for turborepo_repository::package_manager::PackageManager {
        fn from(pm: PackageManager) -> Self {
            match pm {
                PackageManager::Npm => Self::Npm,
                PackageManager::Yarn => Self::Yarn,
                PackageManager::Berry => Self::Berry,
                PackageManager::Pnpm => Self::Pnpm,
                PackageManager::Pnpm6 => Self::Pnpm6,
                PackageManager::Pnpm9 => Self::Pnpm9,
                PackageManager::Bun => Self::Bun,
                // The wire format does not carry nub's underlying lockfile
                // manager. Default to npm's lockfile here; the daemon re-resolves
                // the concrete package manager from disk on the server side.
                PackageManager::Nub => Self::Nub {
                    lockfile: Box::new(Self::Npm),
                },
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
                turborepo_repository::package_manager::PackageManager::Pnpm9 => Self::Pnpm9,
                turborepo_repository::package_manager::PackageManager::Bun => Self::Bun,
                turborepo_repository::package_manager::PackageManager::Nub { .. } => Self::Nub,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use turbopath::AbsoluteSystemPathBuf;

    use super::repo_hash;

    #[test]
    fn test_repo_hash() {
        #[cfg(not(target_os = "windows"))]
        let (path, expected_hash) = ("/tmp/turborepo", "6e0cfa616f75a61c");
        #[cfg(target_os = "windows")]
        let (path, expected_hash) = ("C:\\\\tmp\\turborepo", "0103736e6883e35f");
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let hash = repo_hash(&repo_root);

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn repo_hash_normalizes_drive_letter_case() {
        let lower = AbsoluteSystemPathBuf::new("c:\\tmp\\turborepo").unwrap();
        let upper = AbsoluteSystemPathBuf::new("C:\\tmp\\turborepo").unwrap();

        assert_eq!(repo_hash(&lower), repo_hash(&upper));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn repo_hash_input_preserves_non_drive_paths() {
        let repo_root = AbsoluteSystemPathBuf::new(r"\\server\share\turborepo").unwrap();

        assert_eq!(super::repo_hash_input(&repo_root), repo_root.to_string());
    }
}
