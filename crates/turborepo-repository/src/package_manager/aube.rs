//! aube support.
//!
//! aube (<https://aube.jdx.dev>) reads and writes existing package-manager
//! lockfiles in place, and uses `aube-lock.yaml` for new aube-first projects.
//! Turborepo keeps aube's CLI identity while delegating lockfile operations to
//! the concrete lockfile format present in the repository.

use turbopath::AbsoluteSystemPath;

use crate::package_manager::{PackageManager, bun, npm, pnpm, yarn, yarn::YarnDetector};

pub const LOCKFILE: &str = "aube-lock.yaml";
pub const WORKSPACE_CONFIGURATION_PATH: &str = "aube-workspace.yaml";

/// Determine which package manager's lockfile is present in the repository
/// root, using aube's documented write precedence. Defaults to npm when no
/// lockfile is found yet because npm-family lockfile operations are the least
/// surprising fallback for an empty project.
pub fn underlying_lockfile_manager(repo_root: &AbsoluteSystemPath) -> PackageManager {
    if repo_root.join_component(LOCKFILE).exists() {
        repo_root
            .join_component(LOCKFILE)
            .read()
            .map(|contents| pnpm::detect_from_lockfile_contents(&contents))
            .unwrap_or(PackageManager::Pnpm9)
    } else if repo_root.join_component(pnpm::LOCKFILE).exists() {
        pnpm::detect_from_lockfile(repo_root).unwrap_or(PackageManager::Pnpm)
    } else if repo_root.join_component(bun::LOCKFILE).exists() {
        PackageManager::Bun
    } else if repo_root.join_component(yarn::LOCKFILE).exists() {
        YarnDetector::new(repo_root)
            .detect_from_lockfile()
            .unwrap_or(PackageManager::Yarn)
    } else if repo_root.join_component(npm::LOCKFILE).exists() {
        PackageManager::Npm
    } else {
        PackageManager::Npm
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;
    use crate::package_manager::{PackageManager, pnpm};

    #[test]
    fn test_aube_underlying_lockfile_resolution() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        assert_eq!(underlying_lockfile_manager(&repo_root), PackageManager::Npm);

        repo_root.join_component(pnpm::LOCKFILE).create().unwrap();
        assert_eq!(
            underlying_lockfile_manager(&repo_root),
            PackageManager::Pnpm
        );

        repo_root
            .join_component(LOCKFILE)
            .create_with_contents("lockfileVersion: '9.0'\n")
            .unwrap();
        assert_eq!(
            underlying_lockfile_manager(&repo_root),
            PackageManager::Pnpm9
        );
    }
}
