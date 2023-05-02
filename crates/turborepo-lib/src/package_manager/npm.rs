use anyhow::Result;
use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use crate::package_manager::PackageManager;

pub const LOCKFILE: &str = "package-lock.json";

pub struct NpmDetector<'a> {
    repo_root: &'a AbsoluteSystemPathBuf,
    found: bool,
}

impl<'a> NpmDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPathBuf) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }
}

impl<'a> Iterator for NpmDetector<'a> {
    type Item = Result<PackageManager>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }

        self.found = true;
        let package_json = self
            .repo_root
            .join_relative(RelativeSystemPathBuf::new(LOCKFILE).unwrap());

        if package_json.exists() {
            Some(Ok(PackageManager::Npm))
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

    use super::LOCKFILE;
    use crate::{commands::CommandBase, get_version, package_manager::PackageManager, Args};

    #[test]
    fn test_detect_npm() -> Result<()> {
        let repo_root = tempdir()?;
        let mut base = CommandBase::new(
            Args::default(),
            repo_root.path().to_path_buf(),
            get_version(),
        )?;

        let lockfile_path = repo_root.path().join(LOCKFILE);
        File::create(&lockfile_path)?;
        let package_manager = PackageManager::detect_package_manager(&mut base)?;
        assert_eq!(package_manager, PackageManager::Npm);

        Ok(())
    }
}
