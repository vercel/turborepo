//! nub support.
//!
//! nub (<https://nub.dev>) is a Rust CLI that augments the user's installed
//! Node and ships a pnpm-compatible package-manager command surface. Unlike the
//! other supported package managers, nub does not define its own lockfile
//! format: it is lockfile-compatible with whatever the project already uses
//! (npm, pnpm, yarn, or bun). For that reason nub is recognized through the
//! `packageManager` field in `package.json` (`"nub@x.y.z"`) or nub's native
//! `lock.yaml`; a bare foreign lockfile alone does not imply nub.
//!
//! Because Turborepo needs a parsed lockfile for pruning and cache hashing, the
//! [`PackageManager::Nub`] variant carries the concrete package manager whose
//! lockfile is present in the repository, and lockfile-related operations
//! delegate to it.

use turbopath::AbsoluteSystemPath;

use crate::package_manager::{PackageManager, bun, pnpm, yarn, yarn::YarnDetector};

pub const LOCKFILE: &str = "lock.yaml";

/// Determine which package manager's lockfile is present in the repository
/// root, in priority order. Defaults to npm when no lockfile is found yet (e.g.
/// a fresh nub project before the first install), matching nub's npm-compatible
/// default lockfile.
pub fn underlying_lockfile_manager(repo_root: &AbsoluteSystemPath) -> PackageManager {
    if repo_root.join_component(bun::LOCKFILE).exists() {
        PackageManager::Bun
    } else if repo_root.join_component(LOCKFILE).exists() {
        repo_root
            .join_component(LOCKFILE)
            .read()
            .map(|contents| pnpm::detect_from_lockfile_contents(&contents))
            .unwrap_or(PackageManager::Pnpm)
    } else if repo_root.join_component(pnpm::LOCKFILE).exists() {
        pnpm::detect_from_lockfile(repo_root).unwrap_or(PackageManager::Pnpm)
    } else if repo_root.join_component(yarn::LOCKFILE).exists() {
        YarnDetector::new(repo_root)
            .detect_from_lockfile()
            .unwrap_or(PackageManager::Yarn)
    } else {
        // npm's lockfile, or no lockfile at all -> treat as npm.
        PackageManager::Npm
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_errors::Spanned;

    use super::*;
    use crate::{
        package_json::PackageJson,
        package_manager::{PackageManager, npm},
    };

    #[test]
    fn test_nub_detected_only_via_package_manager_field() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        // A bare npm lockfile alone must NOT be detected as nub.
        repo_root.join_component(npm::LOCKFILE).create().unwrap();
        let detected = PackageManager::detect_package_manager(&repo_root).unwrap();
        assert_eq!(detected, PackageManager::Npm);

        // A foreign lockfile alone must not imply nub.
        let package_json = PackageJson {
            package_manager: Some(Spanned::new("nub@0.1.0".to_string())),
            ..Default::default()
        };
        let pm = PackageManager::read_or_detect_package_manager(&package_json, &repo_root).unwrap();
        assert!(matches!(pm, PackageManager::Nub { .. }));
    }

    #[test]
    fn test_nub_underlying_lockfile_resolution() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        // No lockfile yet -> npm default.
        assert_eq!(underlying_lockfile_manager(&repo_root), PackageManager::Npm);

        repo_root.join_component(pnpm::LOCKFILE).create().unwrap();
        assert_eq!(
            underlying_lockfile_manager(&repo_root),
            PackageManager::Pnpm
        );

        // bun takes priority when multiple are present.
        repo_root.join_component(bun::LOCKFILE).create().unwrap();
        assert_eq!(underlying_lockfile_manager(&repo_root), PackageManager::Bun);
    }

    #[test]
    fn test_nub_underlying_lockfile_detects_berry() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        repo_root
            .join_component(yarn::LOCKFILE)
            .create_with_contents(
                "__metadata:\n  version: 6\n  cacheKey: 8\n\n\"@pkg/foo@npm:1.0.0\":\n  version: \
                 1.0.0\n",
            )
            .unwrap();

        assert_eq!(
            underlying_lockfile_manager(&repo_root),
            PackageManager::Berry
        );
    }

    #[test]
    fn test_nub_underlying_lockfile_detects_pnpm9() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        repo_root
            .join_component(pnpm::LOCKFILE)
            .create_with_contents("lockfileVersion: '9.0'\n")
            .unwrap();

        assert_eq!(
            underlying_lockfile_manager(&repo_root),
            PackageManager::Pnpm9
        );
    }

    #[test]
    fn test_nub_underlying_native_lockfile_detects_pnpm9() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        repo_root
            .join_component(LOCKFILE)
            .create_with_contents("lockfileVersion: '9.0'\n")
            .unwrap();

        assert_eq!(
            underlying_lockfile_manager(&repo_root),
            PackageManager::Pnpm9
        );
    }

    #[test]
    fn test_nub_detected_from_native_lockfile() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        repo_root
            .join_component(LOCKFILE)
            .create_with_contents("lockfileVersion: '9.0'\n")
            .unwrap();

        assert_eq!(
            PackageManager::detect_package_manager(&repo_root).unwrap(),
            PackageManager::Nub {
                lockfile: Box::new(PackageManager::Pnpm9)
            }
        );
    }
}
