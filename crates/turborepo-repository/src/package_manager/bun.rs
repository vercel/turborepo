use turbopath::AbsoluteSystemPath;

use crate::package_manager::{Error, PackageManager};

pub const LOCKFILE: &str = "bun.lockb";

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

impl<'a> Iterator for BunDetector<'a> {
    type Item = Result<PackageManager, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }

        self.found = true;
        let package_json = self.repo_root.join_component(LOCKFILE);

        if package_json.exists() {
            Some(Ok(PackageManager::Bun))
        } else {
            None
        }
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
    fn test_detect_bun() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;

        let lockfile_path = repo_root.path().join(LOCKFILE);
        File::create(lockfile_path)?;
        let package_manager = PackageManager::detect_package_manager(&repo_root_path)?;
        assert_eq!(package_manager, PackageManager::Bun);

        Ok(())
    }
}
