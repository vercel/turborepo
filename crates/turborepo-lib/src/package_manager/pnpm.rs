use anyhow::Result;
use node_semver::{Range, Version};
use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use crate::package_manager::PackageManager;

pub const LOCKFILE: &str = "pnpm-lock.yaml";

pub struct PnpmDetector<'a> {
    found: bool,
    repo_root: &'a AbsoluteSystemPathBuf,
}

impl<'a> PnpmDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPathBuf) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }

    pub fn detect_pnpm6_or_pnpm(version: &Version) -> Result<PackageManager> {
        let pnpm6_constraint: Range = "<7.0.0".parse()?;
        if pnpm6_constraint.satisfies(version) {
            Ok(PackageManager::Pnpm6)
        } else {
            Ok(PackageManager::Pnpm)
        }
    }
}

impl<'a> Iterator for PnpmDetector<'a> {
    type Item = Result<PackageManager>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }
        self.found = true;

        let pnpm_lockfile = self
            .repo_root
            .join_relative(RelativeSystemPathBuf::new(LOCKFILE).unwrap());

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
    use crate::{commands::CommandBase, get_version, package_manager::PackageManager, Args};

    #[test]
    fn test_detect_pnpm() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::new(repo_root.path())?;
        let mut base = CommandBase::new(Args::default(), repo_root_path, get_version())?;

        let lockfile_path = repo_root.path().join(LOCKFILE);
        File::create(&lockfile_path)?;
        let package_manager = PackageManager::detect_package_manager(&mut base)?;
        assert_eq!(package_manager, PackageManager::Pnpm);

        Ok(())
    }
}
