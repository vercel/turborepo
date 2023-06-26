use node_semver::{Range, Version};
use turbopath::AbsoluteSystemPath;

use crate::package_manager::{Error, PackageManager};

pub const LOCKFILE: &str = "pnpm-lock.yaml";

pub struct PnpmDetector<'a> {
    found: bool,
    repo_root: &'a AbsoluteSystemPath,
}

impl<'a> PnpmDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }

    pub fn detect_pnpm6_or_pnpm(version: &Version) -> Result<PackageManager, Error> {
        let pnpm6_constraint: Range = "<7.0.0".parse()?;
        if pnpm6_constraint.satisfies(version) {
            Ok(PackageManager::Pnpm6)
        } else {
            Ok(PackageManager::Pnpm)
        }
    }
}

impl<'a> Iterator for PnpmDetector<'a> {
    type Item = Result<PackageManager, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }
        self.found = true;

        let pnpm_lockfile = self.repo_root.join_component(LOCKFILE);

        pnpm_lockfile.exists().then(|| Ok(PackageManager::Pnpm))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::LOCKFILE;
    use crate::package_manager::PackageManager;

    #[test]
    fn test_detect_pnpm() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        let lockfile_path = repo_root.path().join(LOCKFILE);
        File::create(lockfile_path)?;
        let package_manager = PackageManager::detect_package_manager(&repo_root_path)?;
        assert_eq!(package_manager, PackageManager::Pnpm);

        Ok(())
    }
}
