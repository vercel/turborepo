//! utoo support.
//!
//! utoo (<https://utoo.land>) is an npm-compatible package manager exposed by
//! the `ut` CLI. It uses npm workspace declarations and `package-lock.json`, so
//! Turborepo keeps utoo's CLI identity while delegating lockfile operations to
//! npm.

use turbopath::AbsoluteSystemPath;

use crate::package_manager::PackageManager;

/// utoo uses npm-compatible lockfile semantics. A bare `package-lock.json`
/// should continue to detect as npm, so utoo is resolved only from package
/// manager declarations.
pub fn underlying_lockfile_manager(_repo_root: &AbsoluteSystemPath) -> PackageManager {
    PackageManager::Npm
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
    fn test_utoo_detected_only_via_package_manager_field() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        repo_root.join_component(npm::LOCKFILE).create().unwrap();
        let detected = PackageManager::detect_package_manager(&repo_root).unwrap();
        assert_eq!(detected, PackageManager::Npm);

        let package_json = PackageJson {
            package_manager: Some(Spanned::new("utoo@1.1.3".to_string())),
            ..Default::default()
        };
        let pm = PackageManager::read_or_detect_package_manager(&package_json, &repo_root).unwrap();
        assert_eq!(
            pm,
            PackageManager::Utoo {
                lockfile: Box::new(PackageManager::Npm)
            }
        );
    }

    #[test]
    fn test_utoo_underlying_lockfile_resolution() {
        let dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path()).unwrap();

        assert_eq!(underlying_lockfile_manager(&repo_root), PackageManager::Npm);
    }
}
