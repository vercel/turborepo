use turbopath::AbsoluteSystemPath;

use crate::package_manager::{Error, PackageManager};

pub const LOCKFILE: &str = "bun.lock";
pub const LOCKFILE_BINARY: &str = "bun.lockb";

pub struct BunDetector<'a> {
    repo_root: &'a AbsoluteSystemPath,
    found: bool,
}

impl<'a> BunDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }
}

impl Iterator for BunDetector<'_> {
    type Item = Result<PackageManager, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }

        self.found = true;
        let bun_lock = self.repo_root.join_component(LOCKFILE);

        if bun_lock.exists() {
            Some(Ok(PackageManager::Bun))
        } else if self.repo_root.join_component(LOCKFILE_BINARY).exists() {
            Some(Err(Error::BunBinaryLockfile))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    #[test]
    fn test_detect_bun() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;

        repo_root_path.join_component(LOCKFILE).create()?;
        let package_manager = PackageManager::detect_package_manager(&repo_root_path)?;
        assert_eq!(package_manager, PackageManager::Bun);

        Ok(())
    }

    #[test]
    fn test_detect_bun_binary() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;

        repo_root_path.join_component(LOCKFILE_BINARY).create()?;
        let package_manager = PackageManager::detect_package_manager(&repo_root_path).unwrap_err();
        assert_matches!(package_manager, Error::BunBinaryLockfile);
        Ok(())
    }

    #[test]
    fn test_detect_bun_both() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;

        repo_root_path.join_component(LOCKFILE).create()?;
        repo_root_path.join_component(LOCKFILE_BINARY).create()?;
        let package_manager = PackageManager::detect_package_manager(&repo_root_path)?;
        assert_eq!(package_manager, PackageManager::Bun);
        Ok(())
    }
}
