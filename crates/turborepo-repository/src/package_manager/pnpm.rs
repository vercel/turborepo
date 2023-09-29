use std::collections::HashSet;

use node_semver::{Range, Version};
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};

use crate::{
    package_json::PackageJson,
    package_manager::{Error, PackageManager},
};

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

pub(crate) fn prune_patches<R: AsRef<RelativeUnixPath>>(
    package_json: &PackageJson,
    patches: &[R],
) -> PackageJson {
    let mut pruned_json = package_json.clone();
    let patches = patches.iter().map(|r| r.as_ref()).collect::<HashSet<_>>();

    if let Some(existing_patches) = pruned_json
        .pnpm
        .as_mut()
        .and_then(|config| config.patched_dependencies.as_mut())
    {
        existing_patches.retain(|_, patch_path| patches.contains(patch_path.as_ref()));
    }

    pruned_json
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

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use serde_json::json;
    use turbopath::RelativeUnixPathBuf;

    use super::*;

    #[test]
    fn test_patch_pruning() {
        let package_json: PackageJson = serde_json::from_value(json!({
            "name": "pnpm-patches",
            "pnpm": {
                "patchedDependencies": {
                    "foo@1.0.0": "patches/foo@1.0.0.patch",
                    "bar@1.2.3": "patches/bar@1.2.3.patch",
                }
            }
        }))
        .unwrap();
        let patches = vec![RelativeUnixPathBuf::new("patches/foo@1.0.0.patch").unwrap()];
        let pruned = prune_patches(&package_json, &patches);
        assert_eq!(
            pruned
                .pnpm
                .as_ref()
                .and_then(|c| c.patched_dependencies.as_ref()),
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
