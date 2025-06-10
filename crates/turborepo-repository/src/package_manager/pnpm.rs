use std::collections::HashSet;

use node_semver::{Range, Version};
use serde::Deserialize;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};

use super::npmrc;
use crate::{
    package_json::PackageJson,
    package_manager::{Error, PackageManager},
};

pub const LOCKFILE: &str = "pnpm-lock.yaml";
pub const WORKSPACE_CONFIGURATION_PATH: &str = "pnpm-workspace.yaml";

/// A representation of the pnpm versions have different treatment by turbo.
///
/// Not all behaviors are gated by this enum, lockfile interpretations are
/// decided by `lockfileVersion` in `pnpm-lock.yaml`. In the future, this would
/// be better represented by the semver to allow better gating of behavior
/// based on when it changed in pnpm.
pub enum PnpmVersion {
    Pnpm6,
    Pnpm7And8,
    Pnpm9,
}

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
        let pnpm6_constraint: Range = "<7.0.0".parse().expect("valid version");
        let pnpm9_constraint: Range = ">=9.0.0-alpha.0".parse().expect("valid version");
        if pnpm6_constraint.satisfies(version) {
            Ok(PackageManager::Pnpm6)
        } else if pnpm9_constraint.satisfies(version) {
            Ok(PackageManager::Pnpm9)
        } else {
            Ok(PackageManager::Pnpm)
        }
    }
}

impl Iterator for PnpmDetector<'_> {
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
    repo_root: &AbsoluteSystemPath,
) -> PackageJson {
    let mut pruned_json = package_json.clone();
    let patches_set = patches.iter().map(|r| r.as_ref()).collect::<HashSet<_>>();

    if let Some(existing_patches) = pruned_json
        .pnpm
        .as_mut()
        .and_then(|config| config.patched_dependencies.as_mut())
    {
        existing_patches.retain(|_, patch_path| patches_set.contains(patch_path.as_ref()));
    }

    // Patches can be declared in pnpm-workspace.yaml as well
    if let Ok(workspace) = PnpmWorkspace::from_file(repo_root) {
        let pnpm_config = pruned_json.pnpm.get_or_insert_with(Default::default);
        let patched_deps = pnpm_config
            .patched_dependencies
            .get_or_insert_with(Default::default);

        for (key, patch_path) in workspace.patched_dependencies.into_iter().flatten() {
            if patches_set.contains(patch_path.as_ref()) {
                patched_deps.insert(key, patch_path);
            }
        }
    }

    pruned_json
}

pub fn link_workspace_packages(pnpm_version: PnpmVersion, repo_root: &AbsoluteSystemPath) -> bool {
    let npmrc_config = npmrc::NpmRc::from_file(repo_root)
        .inspect_err(|e| debug!("unable to read npmrc: {e}"))
        .unwrap_or_default();
    let workspace_config = matches!(pnpm_version, PnpmVersion::Pnpm9)
        .then(|| {
            PnpmWorkspace::from_file(repo_root)
                .inspect_err(|e| debug!("unable to read {WORKSPACE_CONFIGURATION_PATH}: {e}"))
                .ok()
        })
        .flatten()
        .and_then(|config| config.link_workspace_packages());
    workspace_config
        .or(npmrc_config.link_workspace_packages)
        // The default for pnpm 9 is false if not explicitly set
        // All previous versions had a default of true
        .unwrap_or(match pnpm_version {
            PnpmVersion::Pnpm6 | PnpmVersion::Pnpm7And8 => true,
            PnpmVersion::Pnpm9 => false,
        })
}

pub fn get_configured_workspace_globs(repo_root: &AbsoluteSystemPath) -> Option<Vec<String>> {
    let pnpm_workspace = PnpmWorkspace::from_file(repo_root).ok()?;
    if pnpm_workspace.packages.is_empty() {
        None
    } else {
        Some(pnpm_workspace.packages)
    }
}

pub fn get_default_exclusions() -> &'static [&'static str] {
    ["**/node_modules/**", "**/bower_components/**"].as_slice()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PnpmWorkspace {
    pub packages: Vec<String>,
    link_workspace_packages: Option<LinkWorkspacePackages>,
    pub patched_dependencies:
        Option<std::collections::BTreeMap<String, turbopath::RelativeUnixPathBuf>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LinkWorkspacePackages {
    Bool(bool),
    Str(String),
}

impl PnpmWorkspace {
    pub fn from_file(repo_root: &AbsoluteSystemPath) -> Result<Self, Error> {
        let workspace_yaml_path = repo_root.join_component(WORKSPACE_CONFIGURATION_PATH);
        let workspace_yaml = workspace_yaml_path.read_to_string()?;
        Ok(serde_yaml::from_str(&workspace_yaml)?)
    }

    fn link_workspace_packages(&self) -> Option<bool> {
        let config = self.link_workspace_packages.as_ref()?;
        match config {
            LinkWorkspacePackages::Bool(value) => Some(*value),
            LinkWorkspacePackages::Str(value) => Some(value == "deep"),
        }
    }
}

#[derive(Debug)]
pub struct NotPnpmError {
    package_manager: PackageManager,
}

impl std::fmt::Display for NotPnpmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "Package managers other than pnpm cannot have pnpm version: {:?}",
            self.package_manager
        ))
    }
}

impl TryFrom<&'_ PackageManager> for PnpmVersion {
    type Error = NotPnpmError;

    fn try_from(value: &PackageManager) -> Result<Self, Self::Error> {
        match value {
            PackageManager::Pnpm9 => Ok(Self::Pnpm9),
            PackageManager::Pnpm => Ok(Self::Pnpm7And8),
            PackageManager::Pnpm6 => Ok(Self::Pnpm6),
            PackageManager::Berry
            | PackageManager::Yarn
            | PackageManager::Npm
            | PackageManager::Bun => Err(NotPnpmError {
                package_manager: value.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod test {
    use std::{collections::BTreeMap, fs::File};

    use serde_json::json;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};

    use super::*;

    #[test]
    fn test_detect_pnpm() -> Result<(), Error> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::try_from(repo_root.path())?;
        let lockfile_path = repo_root.path().join(LOCKFILE);
        File::create(lockfile_path)?;
        let package_manager = PackageManager::detect_package_manager(&repo_root_path)?;
        assert_eq!(package_manager, PackageManager::Pnpm);

        Ok(())
    }

    #[test]
    fn test_patch_pruning() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let package_json: PackageJson = PackageJson::from_value(json!({
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
        let pruned = prune_patches(&package_json, &patches, repo_root);
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

    #[test]
    fn test_workspace_patches_pruning() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();

        let package_json = PackageJson::from_value(json!({
            "name": "pnpm-patches",
        }))
        .unwrap();

        repo_root
            .join_component(WORKSPACE_CONFIGURATION_PATH)
            .create_with_contents(
                "packages:\n  - \"packages/*\"\npatchedDependencies:\n  foo@1.0.0: \
                 patches/foo@1.0.0.patch\n  bar: patches/bar.patch\n",
            )
            .unwrap();
        let patches = vec![RelativeUnixPathBuf::new("patches/foo@1.0.0.patch").unwrap()];
        let pruned = prune_patches(&package_json, &patches, repo_root);
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

    #[test_case("6.0.0", PackageManager::Pnpm6)]
    #[test_case("7.0.0", PackageManager::Pnpm)]
    #[test_case("8.0.0", PackageManager::Pnpm)]
    #[test_case("9.0.0", PackageManager::Pnpm9)]
    #[test_case("9.0.0-alpha.0", PackageManager::Pnpm9)]
    fn test_version_detection(version: &str, expected: PackageManager) {
        let version = Version::parse(version).unwrap();
        assert_eq!(
            PnpmDetector::detect_pnpm6_or_pnpm(&version).unwrap(),
            expected
        );
    }

    #[test]
    fn test_workspace_parsing() {
        let config: PnpmWorkspace =
            serde_yaml::from_str("linkWorkspacePackages: true\npackages:\n  - \"apps/*\"\n")
                .unwrap();
        assert_eq!(config.link_workspace_packages(), Some(true));
        assert_eq!(config.packages, vec!["apps/*".to_string()]);
    }

    #[test_case(PnpmVersion::Pnpm6, None, true)]
    #[test_case(PnpmVersion::Pnpm7And8, None, true)]
    #[test_case(PnpmVersion::Pnpm7And8, Some(false), false)]
    #[test_case(PnpmVersion::Pnpm7And8, Some(true), true)]
    #[test_case(PnpmVersion::Pnpm9, None, false)]
    #[test_case(PnpmVersion::Pnpm9, Some(true), true)]
    #[test_case(PnpmVersion::Pnpm9, Some(false), false)]
    fn test_link_workspace_packages(version: PnpmVersion, enabled: Option<bool>, expected: bool) {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        if let Some(enabled) = enabled {
            repo_root
                .join_component(npmrc::NPMRC_FILENAME)
                .create_with_contents(format!("link-workspace-packages={enabled}"))
                .unwrap();
        }
        let actual = link_workspace_packages(version, repo_root);
        assert_eq!(actual, expected);
    }

    #[test_case(PnpmVersion::Pnpm6, None, true)]
    #[test_case(PnpmVersion::Pnpm7And8, None, true)]
    // Pnpm <9 doesn't use workspace config
    #[test_case(PnpmVersion::Pnpm7And8, Some(false), true)]
    #[test_case(PnpmVersion::Pnpm7And8, Some(true), true)]
    #[test_case(PnpmVersion::Pnpm9, None, false)]
    #[test_case(PnpmVersion::Pnpm9, Some(true), true)]
    #[test_case(PnpmVersion::Pnpm9, Some(false), false)]
    fn test_link_workspace_packages_via_workspace(
        version: PnpmVersion,
        enabled: Option<bool>,
        expected: bool,
    ) {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        if let Some(enabled) = enabled {
            repo_root
                .join_component(WORKSPACE_CONFIGURATION_PATH)
                .create_with_contents(format!(
                    "linkWorkspacePackages: {enabled}\npackages:\n  - \"apps/*\"\n"
                ))
                .unwrap();
        }
        let actual = link_workspace_packages(version, repo_root);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_workspace_yaml_wins_over_npmrc() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        repo_root
            .join_component(WORKSPACE_CONFIGURATION_PATH)
            .create_with_contents("linkWorkspacePackages: true\npackages:\n  - \"apps/*\"\n")
            .unwrap();
        repo_root
            .join_component(npmrc::NPMRC_FILENAME)
            .create_with_contents("link-workspace-packages=false")
            .unwrap();
        let actual = link_workspace_packages(PnpmVersion::Pnpm9, repo_root);
        assert!(actual);
    }

    #[test]
    fn test_workspace_yaml_supports_deep() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        repo_root
            .join_component(WORKSPACE_CONFIGURATION_PATH)
            .create_with_contents("linkWorkspacePackages: deep\npackages:\n  - \"apps/*\"\n")
            .unwrap();
        let actual = link_workspace_packages(PnpmVersion::Pnpm9, repo_root);
        assert!(actual, "deep should be treated as true");
    }
}
