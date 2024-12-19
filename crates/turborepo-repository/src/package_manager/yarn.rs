use std::process::Command;

use miette::NamedSource;
use node_semver::{Range, Version};
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};
use which::which;

use crate::{
    package_json::PackageJson,
    package_manager::{Error, PackageManager},
};

pub const LOCKFILE: &str = "yarn.lock";

pub struct YarnDetector<'a> {
    repo_root: &'a AbsoluteSystemPath,
    found: bool,
}

impl<'a> YarnDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self {
            repo_root,
            found: false,
        }
    }

    fn get_yarn_version(&self) -> Result<Version, Error> {
        let binary = "yarn";
        let yarn_binary = which(binary).map_err(|e| Error::Which(e, binary.to_string()))?;
        let output = Command::new(yarn_binary)
            .arg("--version")
            .current_dir(self.repo_root)
            .output()?;
        let yarn_version_output = String::from_utf8(output.stdout)?;
        yarn_version_output
            .trim()
            .parse()
            .map_err(|err| Error::InvalidVersion {
                explanation: format!("{} {}", yarn_version_output, err),
                span: None,
                text: NamedSource::new("yarn --version", yarn_version_output),
            })
    }

    pub fn detect_berry_or_yarn(version: &Version) -> Result<PackageManager, Error> {
        let berry_constraint: Range = ">=2.0.0-0".parse().expect("valid version");
        if berry_constraint.satisfies(version) {
            Ok(PackageManager::Berry)
        } else {
            Ok(PackageManager::Yarn)
        }
    }
}

pub(crate) fn prune_patches<R: AsRef<RelativeUnixPath>>(
    package_json: &PackageJson,
    patches: &[R],
) -> PackageJson {
    let mut pruned_json = package_json.clone();
    let patches = patches
        .iter()
        .map(|patch_path| patch_path.as_ref().to_string())
        .collect::<Vec<_>>();

    if let Some(existing_patches) = &mut pruned_json.resolutions {
        existing_patches.retain(|_, resolution| {
            // Keep any non-patch resolution entries
            !resolution.ends_with(".patch")
                // Keep any patch resolutions if they end with a patch file path
                || patches.iter().any(|patch| resolution.ends_with(patch))
        })
    }

    pruned_json
}

impl<'a> Iterator for YarnDetector<'a> {
    type Item = Result<PackageManager, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }
        self.found = true;

        let yarn_lockfile = self.repo_root.join_component(LOCKFILE);

        if yarn_lockfile.exists() {
            Some(
                self.get_yarn_version()
                    .and_then(|version| Self::detect_berry_or_yarn(&version)),
            )
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use anyhow::Result;
    use serde_json::json;
    use turbopath::RelativeUnixPathBuf;

    use super::prune_patches;
    use crate::{
        package_json::PackageJson,
        package_manager::{yarn::YarnDetector, PackageManager},
    };

    #[test]
    fn test_detect_yarn() -> Result<()> {
        let package_manager = YarnDetector::detect_berry_or_yarn(&"1.22.10".parse()?)?;
        assert_eq!(package_manager, PackageManager::Yarn);

        let package_manager = YarnDetector::detect_berry_or_yarn(&"2.22.10".parse()?)?;
        assert_eq!(package_manager, PackageManager::Berry);

        Ok(())
    }

    #[test]
    fn test_patch_pruning() {
        let package_json: PackageJson = PackageJson::from_value(json!({
            "name": "pnpm-patches",
            "resolutions": {
                "foo@1.0.0": "patch:foo@npm%3A1.0.0#./.yarn/patches/foo-npm-1.0.0-time.patch",
                "bar@1.2.3": "patch:bar@npm%3A1.2.3#./.yarn/patches/bar-npm-1.2.3-time.patch",
                "baz": "1.0.0",
            }
        }))
        .unwrap();
        let patches =
            vec![RelativeUnixPathBuf::new(".yarn/patches/foo-npm-1.0.0-time.patch").unwrap()];
        let pruned = prune_patches(&package_json, &patches);
        assert_eq!(
            pruned.resolutions.as_ref(),
            Some(
                [
                    (
                        "foo@1.0.0",
                        "patch:foo@npm%3A1.0.0#./.yarn/patches/foo-npm-1.0.0-time.patch"
                    ),
                    // Should be kept as it isn't a patch, but a version override
                    ("baz", "1.0.0")
                ]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>()
            )
            .as_ref()
        );
    }
}
