use std::collections::HashSet;

use turbopath::{AbsoluteSystemPath, RelativeUnixPath};

use crate::{
    package_json::PackageJson,
    package_manager::{Error, PackageManager},
};

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
    use std::{assert_matches::assert_matches, collections::BTreeMap};

    use anyhow::Result;
    use serde_json::json;
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};

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

    #[test]
    fn test_patch_pruning() {
        let package_json: PackageJson = PackageJson::from_value(json!({
            "name": "bun-patches",
            "patchedDependencies": {
                "foo@1.0.0": "patches/foo@1.0.0.patch",
                "bar@1.2.3": "patches/bar@1.2.3.patch",
            }
        }))
        .unwrap();
        let patches = vec![RelativeUnixPathBuf::new("patches/foo@1.0.0.patch").unwrap()];
        let pruned = prune_patches(&package_json, &patches);
        assert_eq!(
            pruned.patched_dependencies.as_ref(),
            Some(
                [("foo@1.0.0", "patches/foo@1.0.0.patch")]
                    .iter()
                    .map(|(k, v)| (k.to_string(), RelativeUnixPathBuf::new(*v).unwrap()))
                    .collect::<BTreeMap<_, _>>()
            )
            .as_ref()
        );
    }
}
