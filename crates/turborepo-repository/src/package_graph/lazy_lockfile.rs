//! Lazy lockfile loading for the package graph cache.
//!
//! When restoring a PackageGraph from cache, we don't need the parsed lockfile
//! immediately. This module spawns the lockfile parse on a background thread
//! so it runs in parallel with other setup work.

use turbopath::AbsoluteSystemPathBuf;
use turborepo_lockfiles::Lockfile;

use crate::{package_json::PackageJson, package_manager::PackageManager};

/// A handle to a lockfile being parsed on a background thread.
pub struct LazyLockfile {
    handle: tokio::task::JoinHandle<Option<Box<dyn Lockfile>>>,
}

impl LazyLockfile {
    /// Spawn lockfile parsing on a background thread.
    pub fn spawn(
        package_manager: PackageManager,
        repo_root: AbsoluteSystemPathBuf,
        root_package_json: PackageJson,
    ) -> Self {
        let handle = tokio::task::spawn_blocking(move || {
            package_manager
                .read_lockfile(&repo_root, &root_package_json)
                .ok()
        });
        Self { handle }
    }

    /// Wait for the lockfile to finish parsing.
    /// Returns `None` if parsing failed or the task panicked.
    pub async fn resolve(self) -> Option<Box<dyn Lockfile>> {
        match self.handle.await {
            Ok(lockfile) => lockfile,
            Err(e) => {
                tracing::warn!("lockfile parse task panicked: {}", e);
                None
            }
        }
    }
}
