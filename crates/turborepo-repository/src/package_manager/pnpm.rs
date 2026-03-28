use std::collections::{HashMap, HashSet};

use node_semver::{Range, Version};
use serde::Deserialize;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath, RelativeUnixPathBuf};

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
    _repo_root: &AbsoluteSystemPath,
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

    pruned_json
}

/// Prune `patchedDependencies` in a `pnpm-workspace.yaml` file in-place,
/// retaining only entries whose patch path is in `patches`.
pub fn prune_workspace_patches<R: AsRef<RelativeUnixPath>>(
    workspace_yaml_path: &AbsoluteSystemPath,
    patches: &[R],
) -> Result<(), std::io::Error> {
    if !workspace_yaml_path.exists() {
        return Ok(());
    }
    let contents = workspace_yaml_path.read_to_string()?;
    let mut doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&contents)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let patches_set: HashSet<&RelativeUnixPath> = patches.iter().map(|r| r.as_ref()).collect();

    if let Some(patched_deps) = doc.get_mut("patchedDependencies")
        && let Some(mapping) = patched_deps.as_mapping_mut()
    {
        mapping.retain(|_key, val| {
            val.as_str()
                .and_then(|s| RelativeUnixPathBuf::new(s).ok())
                .is_some_and(|p| patches_set.contains(p.as_ref()))
        });
    }

    let output = serde_yaml_ng::to_string(&doc)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    workspace_yaml_path.create_with_contents(output)?;
    Ok(())
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
    #[serde(rename = "patchedDependencies")]
    _patched_dependencies:
        Option<std::collections::BTreeMap<String, turbopath::RelativeUnixPathBuf>>,
    /// Default catalog (`catalog:` protocol resolves to these)
    #[serde(default)]
    pub catalog: HashMap<String, String>,
    /// Named catalogs (`catalog:<name>` protocol resolves to these)
    #[serde(default)]
    pub catalogs: HashMap<String, HashMap<String, String>>,
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
        Ok(serde_yaml_ng::from_str(&workspace_yaml)?)
    }

    fn link_workspace_packages(&self) -> Option<bool> {
        let config = self.link_workspace_packages.as_ref()?;
        match config {
            LinkWorkspacePackages::Bool(value) => Some(*value),
            LinkWorkspacePackages::Str(value) => Some(value == "deep"),
        }
    }
}

/// Read catalog definitions from pnpm-workspace.yaml. Returns `None` if the
/// file doesn't exist or can't be parsed (non-fatal).
pub fn read_catalogs(repo_root: &AbsoluteSystemPath) -> Option<PnpmCatalogs> {
    let workspace = PnpmWorkspace::from_file(repo_root)
        .inspect_err(|e| debug!("unable to read {WORKSPACE_CONFIGURATION_PATH}: {e}"))
        .ok()?;
    if workspace.catalog.is_empty() && workspace.catalogs.is_empty() {
        return None;
    }
    Some(PnpmCatalogs {
        default: workspace.catalog,
        named: workspace.catalogs,
    })
}

/// Resolved catalog definitions from pnpm-workspace.yaml.
#[derive(Debug, Default)]
pub struct PnpmCatalogs {
    pub default: HashMap<String, String>,
    pub named: HashMap<String, HashMap<String, String>>,
}

impl PnpmCatalogs {
    /// Resolve a `catalog:` or `catalog:<name>` specifier to the actual
    /// version string. Returns `None` if the specifier is not a catalog
    /// reference or the package isn't found in the catalog.
    pub fn resolve<'a>(&'a self, name: &str, specifier: &str) -> Option<&'a str> {
        let catalog_name = specifier.strip_prefix("catalog:")?;
        let catalog_map = if catalog_name.is_empty() || catalog_name == "default" {
            Some(&self.default)
        } else {
            self.named.get(catalog_name)
        };
        catalog_map.and_then(|m| m.get(name).map(|s| s.as_str()))
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
    fn test_workspace_patches_not_migrated_to_package_json() {
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
        // prune_patches should NOT migrate workspace yaml patches into
        // package.json — that would change its content and invalidate caches.
        assert_eq!(
            pruned
                .pnpm
                .as_ref()
                .and_then(|c| c.patched_dependencies.as_ref()),
            None,
        );
    }

    #[test]
    fn test_prune_workspace_yaml_patches() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let ws_path = repo_root.join_component(WORKSPACE_CONFIGURATION_PATH);
        ws_path
            .create_with_contents(
                "packages:\n  - \"packages/*\"\npatchedDependencies:\n  foo@1.0.0: \
                 patches/foo@1.0.0.patch\n  bar@2.0.0: patches/bar@2.0.0.patch\n",
            )
            .unwrap();

        let patches = vec![RelativeUnixPathBuf::new("patches/foo@1.0.0.patch").unwrap()];
        prune_workspace_patches(&ws_path, &patches).unwrap();

        let result: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&ws_path.read_to_string().unwrap()).unwrap();
        let patched = result["patchedDependencies"].as_mapping().unwrap();
        assert_eq!(patched.len(), 1);
        assert_eq!(
            patched
                .get(serde_yaml_ng::Value::String("foo@1.0.0".into()))
                .and_then(|v| v.as_str()),
            Some("patches/foo@1.0.0.patch"),
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
            serde_yaml_ng::from_str("linkWorkspacePackages: true\npackages:\n  - \"apps/*\"\n")
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

    #[test]
    fn test_workspace_parses_catalogs() {
        let yaml = r#"
packages:
  - "packages/*"
catalog:
  react: "^18.2.0"
  pkg-a: "workspace:*"
catalogs:
  internal:
    pkg-b: "workspace:*"
    pkg-c: "workspace:^"
"#;
        let config: PnpmWorkspace = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.catalog.get("react").unwrap(), "^18.2.0");
        assert_eq!(config.catalog.get("pkg-a").unwrap(), "workspace:*");
        let internal = config.catalogs.get("internal").unwrap();
        assert_eq!(internal.get("pkg-b").unwrap(), "workspace:*");
        assert_eq!(internal.get("pkg-c").unwrap(), "workspace:^");
    }

    #[test]
    fn test_workspace_parses_without_catalogs() {
        let yaml = "packages:\n  - \"packages/*\"\n";
        let config: PnpmWorkspace = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(config.catalog.is_empty());
        assert!(config.catalogs.is_empty());
    }

    #[test]
    fn test_pnpm_catalogs_resolve_default() {
        let catalogs = PnpmCatalogs {
            default: [("react".to_string(), "^18.2.0".to_string())]
                .into_iter()
                .collect(),
            named: HashMap::new(),
        };
        assert_eq!(catalogs.resolve("react", "catalog:"), Some("^18.2.0"));
        assert_eq!(
            catalogs.resolve("react", "catalog:default"),
            Some("^18.2.0")
        );
        assert_eq!(catalogs.resolve("unknown", "catalog:"), None);
    }

    #[test]
    fn test_pnpm_catalogs_resolve_named() {
        let catalogs = PnpmCatalogs {
            default: HashMap::new(),
            named: [(
                "internal".to_string(),
                [("pkg-b".to_string(), "workspace:*".to_string())]
                    .into_iter()
                    .collect(),
            )]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            catalogs.resolve("pkg-b", "catalog:internal"),
            Some("workspace:*")
        );
        assert_eq!(catalogs.resolve("pkg-b", "catalog:nonexistent"), None);
        assert_eq!(catalogs.resolve("unknown", "catalog:internal"), None);
    }

    #[test]
    fn test_pnpm_catalogs_non_catalog_specifier() {
        let catalogs = PnpmCatalogs {
            default: [("react".to_string(), "^18.2.0".to_string())]
                .into_iter()
                .collect(),
            named: HashMap::new(),
        };
        assert_eq!(catalogs.resolve("react", "^18.2.0"), None);
        assert_eq!(catalogs.resolve("react", "workspace:*"), None);
    }

    #[test]
    fn test_read_catalogs_from_file() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        repo_root
            .join_component(WORKSPACE_CONFIGURATION_PATH)
            .create_with_contents(
                "packages:\n  - \"packages/*\"\ncatalog:\n  react: \"^18.2.0\"\ncatalogs:\n  \
                 internal:\n    pkg-b: \"workspace:*\"\n",
            )
            .unwrap();
        let catalogs = read_catalogs(repo_root).expect("should read catalogs");
        assert_eq!(catalogs.resolve("react", "catalog:"), Some("^18.2.0"));
        assert_eq!(
            catalogs.resolve("pkg-b", "catalog:internal"),
            Some("workspace:*")
        );
    }

    #[test]
    fn test_read_catalogs_no_catalogs() {
        let tmpdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        repo_root
            .join_component(WORKSPACE_CONFIGURATION_PATH)
            .create_with_contents("packages:\n  - \"packages/*\"\n")
            .unwrap();
        assert!(read_catalogs(repo_root).is_none());
    }
}
