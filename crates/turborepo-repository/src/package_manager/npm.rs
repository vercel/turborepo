use std::collections::HashSet;

use turbopath::{AbsoluteSystemPath, RelativeUnixPath};

use crate::{
    package_json::PackageJson,
    package_manager::{Error, PackageManager},
};

pub const LOCKFILE: &str = "package-lock.json";

pub struct NpmDetector<'a> {
    repo_root: &'a AbsoluteSystemPath,
    found: bool,
}

impl<'a> NpmDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }
}

impl Iterator for NpmDetector<'_> {
    type Item = Result<PackageManager, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }

        self.found = true;
        let package_json = self.repo_root.join_component(LOCKFILE);

        if package_json.exists() {
            Some(Ok(PackageManager::Npm))
        } else {
            None
        }
    }
}

pub(crate) fn prune_patches<R: AsRef<RelativeUnixPath>>(
    package_json: &PackageJson,
    patches: &[R],
) -> PackageJson {
    let mut pruned_json = package_json.clone();
    let patches_set = patches.iter().map(|r| r.as_ref()).collect::<HashSet<_>>();
    if let Some(existing_patches) = pruned_json.patched_dependencies.as_mut() {
        existing_patches.retain(|_, patch_path| patches_set.contains(patch_path.as_ref()));
    }
    pruned_json
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use anyhow::Result;
    use serde_json::json;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};

    use super::*;
    use crate::package_manager::PackageManager;

    #[test_case(&[]; "no retained patches")]
    #[test_case(&["patches/foo.patch"]; "one retained patch")]
    #[test_case(&["patches/foo.patch", "patches/bar.patch"]; "all retained patches")]
    fn test_patch_pruning(paths: &[&str]) {
        let package_json = PackageJson::from_value(json!({
            "name": "npm-patches",
            "patchedDependencies": {
                "foo@1.0.0": "patches/foo.patch",
                "@scope/bar@2.0.0": "patches/bar.patch"
            }
        }))
        .unwrap();
        let patches: Vec<_> = paths
            .iter()
            .map(|path| RelativeUnixPathBuf::new(*path).unwrap())
            .collect();
        let repo_root = tempfile::tempdir().unwrap();
        let pruned = PackageManager::Npm.prune_patched_packages(
            &package_json,
            &patches,
            &AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap(),
        );
        let actual: HashSet<_> = pruned.patched_dependencies.unwrap().into_values().collect();
        assert_eq!(actual, patches.into_iter().collect());
        assert_eq!(package_json.patched_dependencies.unwrap().len(), 2);
    }

    #[test]
    fn test_detect_npm() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;

        let lockfile_path = repo_root.path().join(LOCKFILE);
        File::create(lockfile_path)?;
        let package_manager = PackageManager::detect_package_manager(&repo_root_path)?;
        assert_eq!(package_manager, PackageManager::Npm);

        Ok(())
    }
}
