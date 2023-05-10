use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::dep_path::DepPath;

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PnpmLockfileData {
    lockfile_version: LockfileVersion,
    never_built_dependencies: Option<Vec<String>>,
    only_built_dependencies: Option<Vec<String>>,
    overrides: Option<Map<String, String>>,
    package_extensions_checksum: Option<String>,
    patched_dependencies: Option<Map<String, PatchFile>>,
    importers: Map<String, ProjectSnapshot>,
    packages: Option<Map<String, PackageSnapshot>>,
    time: Option<Map<String, String>>,
}

#[derive(Debug, PartialEq, Eq)]
struct LockfileVersion {
    version: String,
    format: VersionFormat,
}

#[derive(Debug, PartialEq, Eq)]
enum VersionFormat {
    String,
    Float,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchFile {
    path: String,
    hash: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    #[serde(flatten)]
    dependencies: DependencyInfo,
    dependencies_meta: Option<Map<String, DependenciesMeta>>,
    publish_directory: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", untagged)]
pub enum DependencyInfo {
    #[serde(rename_all = "camelCase")]
    PreV6 {
        specifiers: Option<Map<String, String>>,
        dependencies: Option<Map<String, String>>,
        optional_dependencies: Option<Map<String, String>>,
        dev_dependencies: Option<Map<String, String>>,
    },
    #[serde(rename_all = "camelCase")]
    V6 {
        dependencies: Option<Map<String, Dependency>>,
        optional_dependencies: Option<Map<String, Dependency>>,
        dev_dependencies: Option<Map<String, Dependency>>,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dependency {
    specifier: String,
    version: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PackageSnapshot {
    // can we make this flow?/is it necessary?
    resolution: PackageResolution,
    id: Option<String>,

    name: Option<String>,
    version: Option<String>,

    dependencies: Option<Map<String, String>>,
    optional_dependencies: Option<Map<String, String>>,
    patched: Option<bool>,

    #[serde(flatten)]
    other: Map<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct DependenciesMeta {
    injected: Option<bool>,
    node: Option<String>,
    patch: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PackageResolution {
    #[serde(rename = "type")]
    type_field: Option<String>,
    integrity: Option<String>,
    tarball: Option<String>,
    dir: Option<String>,
    repo: Option<String>,
    commit: Option<String>,
}

impl PnpmLockfileData {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let this = serde_yaml::from_slice(bytes)?;
        Ok(this)
    }

    pub fn patches(&self) -> Vec<String> {
        let mut patches = self
            .patched_dependencies
            .iter()
            .flatten()
            .map(|(_, patch)| patch.path.clone())
            .collect::<Vec<_>>();
        patches.sort();
        patches
    }

    fn get_packages(&self, key: &str) -> Option<&PackageSnapshot> {
        self.packages
            .as_ref()
            .and_then(|packages| packages.get(key))
    }

    fn get_workspace(&self, workspace_path: &str) -> Result<&ProjectSnapshot, crate::Error> {
        let key = match workspace_path {
            // For pnpm, the root is named "."
            "" => ".",
            k => k,
        };
        self.importers
            .get(key)
            .ok_or_else(|| crate::Error::MissingWorkspace(workspace_path.into()))
    }

    fn is_v6(&self) -> bool {
        matches!(self.lockfile_version.format, VersionFormat::String)
    }

    fn format_key(&self, name: &str, version: &str) -> String {
        match self.is_v6() {
            true => format!("/{name}@{version}"),
            false => format!("/{name}/{version}"),
        }
    }

    fn extract_version<'a>(&self, key: &'a str) -> Option<Cow<'a, str>> {
        let dp = DepPath::try_from(key).ok()?;
        if let Some(suffix) = dp.peer_suffix {
            let sep = match self.is_v6() {
                true => "",
                false => "_",
            };
            Some(format!("{}{}{}", dp.version, sep, suffix).into())
        } else {
            Some(dp.version.into())
        }
    }

    fn apply_overrides<'a>(&'a self, name: &str, specifier: &'a str) -> &'a str {
        self.overrides
            .as_ref()
            .and_then(|o| o.get(name))
            .map(|s| s.as_str())
            .unwrap_or(specifier)
    }

    fn resolve_specifier<'a>(
        &'a self,
        workspace_path: &str,
        name: &str,
        specifier: &'a str,
    ) -> Result<Option<&'a str>, crate::Error> {
        let importer = self.get_workspace(workspace_path)?;

        let Some((resolved_specifier, resolved_version)) = importer.dependencies.find_resolution(name) else {
            self.get_packages(&self.format_key(name, specifier));
            return Ok(None)
        };

        let override_specifier = self.apply_overrides(name, specifier);
        if resolved_specifier == override_specifier {
            Ok(Some(resolved_version))
        } else if self
            .get_packages(&self.format_key(name, override_specifier))
            .is_some()
        {
            Ok(Some(override_specifier))
        } else {
            Ok(None)
        }
    }

    pub fn subgraph(
        &self,
        workspace_paths: &[String],
        packages: &[String],
    ) -> Result<Self, crate::Error> {
        let mut pruned_packages = Map::new();
        for package in packages {
            let entry = self
                .get_packages(package.as_str())
                .ok_or_else(|| crate::Error::MissingPackage(package.clone()))?;
            pruned_packages.insert(package.clone(), entry.clone());
        }
        todo!()
    }
}

impl crate::Lockfile for PnpmLockfileData {
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        // Check if version is a key
        if self.get_packages(version).is_some() {
            // TODO no unwrap
            let extracted_version = self.extract_version(version).unwrap();
            return Ok(Some(crate::Package {
                key: version.into(),
                version: extracted_version.into(),
            }));
        }

        let Some(resolved_version) = self.resolve_specifier(workspace_path, name, version)? else {
            return Ok(None)
        };

        let key = self.format_key(name, resolved_version);

        if let Some(pkg) = self.get_packages(&key) {
            Ok(Some(crate::Package {
                key,
                version: pkg
                    .version
                    .clone()
                    .unwrap_or_else(|| resolved_version.to_string()),
            }))
        } else if let Some(pkg) = self.get_packages(resolved_version) {
            Ok(Some(crate::Package {
                key: resolved_version.to_string(),
                version: pkg
                    .version
                    .clone()
                    // TODO avoid unwrap here?
                    .unwrap_or_else(|| self.extract_version(resolved_version).unwrap().to_string()),
            }))
        } else {
            Ok(None)
        }
    }

    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        let Some(entry) = self.packages.as_ref().and_then(|pkgs| pkgs.get(key)) else {
            return Ok(None)
        };
        Ok(Some(
            entry
                .dependencies
                .iter()
                .flatten()
                .chain(entry.optional_dependencies.iter().flatten())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ))
    }
}

impl From<f32> for LockfileVersion {
    fn from(value: f32) -> Self {
        Self {
            version: value.to_string(),
            format: VersionFormat::Float,
        }
    }
}

impl From<String> for LockfileVersion {
    fn from(value: String) -> Self {
        Self {
            version: value,
            format: VersionFormat::String,
        }
    }
}

impl<'de> Deserialize<'de> for LockfileVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrNum {
            Str(String),
            Num(f32),
        }

        Ok(match StringOrNum::deserialize(deserializer)? {
            StringOrNum::Num(x) => LockfileVersion::from(x),
            StringOrNum::Str(s) => LockfileVersion::from(s),
        })
    }
}

impl Serialize for LockfileVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.format {
            VersionFormat::String => serializer.serialize_str(&self.version),
            VersionFormat::Float => serializer.serialize_f32(
                self.version
                    .parse()
                    .expect("Expected lockfile version to be valid f32"),
            ),
        }
    }
}

impl DependencyInfo {
    pub fn find_resolution(&self, dependency: &str) -> Option<(&str, &str)> {
        match self {
            DependencyInfo::PreV6 {
                specifiers,
                dependencies,
                optional_dependencies,
                dev_dependencies,
            } => {
                let specifier = specifiers.as_ref().and_then(|s| s.get(dependency))?;
                let version = Self::get_resolution(dependencies, dependency)
                    .or_else(|| Self::get_resolution(dev_dependencies, dependency))
                    .or_else(|| Self::get_resolution(optional_dependencies, dependency))?;
                Some((specifier, version))
            }
            DependencyInfo::V6 {
                dependencies,
                optional_dependencies,
                dev_dependencies,
            } => Self::get_resolution(dependencies, dependency)
                .or_else(|| Self::get_resolution(dev_dependencies, dependency))
                .or_else(|| Self::get_resolution(optional_dependencies, dependency))
                .map(Dependency::as_tuple),
        }
    }

    fn get_resolution<'a, V>(maybe_map: &'a Option<Map<String, V>>, key: &str) -> Option<&'a V> {
        maybe_map.as_ref().and_then(|maybe_map| maybe_map.get(key))
    }
}

impl Dependency {
    fn as_tuple(&self) -> (&str, &str) {
        let Dependency { specifier, version } = self;
        (specifier, version)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use test_case::test_case;

    const PNPM6: &[u8] = include_bytes!("../../fixtures/pnpm6-workspace.yaml").as_slice();
    const PNPM7: &[u8] = include_bytes!("../../fixtures/pnpm7-workspace.yaml").as_slice();
    const PNPM8: &[u8] = include_bytes!("../../fixtures/pnpm8.yaml").as_slice();
    const PNPM_ABSOLUTE: &[u8] = include_bytes!("../../fixtures/pnpm-absolute.yaml").as_slice();
    const PNPM_ABSOLUTE_V6: &[u8] =
        include_bytes!("../../fixtures/pnpm-absolute-v6.yaml").as_slice();
    const PNPM_PEER: &[u8] = include_bytes!("../../fixtures/pnpm-peer-v6.yaml").as_slice();
    const PNPM_TOP_LEVEL_OVERRIDE: &[u8] =
        include_bytes!("../../fixtures/pnpm-top-level-dupe.yaml").as_slice();
    const PNPM_OVERRIDE: &[u8] = include_bytes!("../../fixtures/pnpm-override.yaml").as_slice();

    use super::*;
    use crate::Lockfile;

    #[test]
    fn test_roundtrip() {
        for fixture in &[PNPM6, PNPM7, PNPM8] {
            let lockfile = PnpmLockfileData::from_bytes(fixture).unwrap();
            let serialized_lockfile = serde_yaml::to_string(&lockfile).unwrap();
            let lockfile_from_serialized =
                serde_yaml::from_slice(serialized_lockfile.as_bytes()).unwrap();
            assert_eq!(lockfile, lockfile_from_serialized);
        }
    }

    #[test]
    fn test_patches() {
        let lockfile =
            PnpmLockfileData::from_bytes(include_bytes!("../../fixtures/pnpm-patch.yaml")).unwrap();
        assert_eq!(
            lockfile.patches(),
            vec![
                "patches/@babel__core@7.20.12.patch".to_string(),
                "patches/is-odd@3.0.1.patch".to_string(),
                "patches/moleculer@0.14.28.patch".to_string(),
            ]
        );
    }

    #[test_case(
        PNPM7,
        "apps/docs",
        "next",
        "12.2.5",
        Ok(Some("12.2.5_ir3quccc6i62x6qn6jjhyjjiey"))
        ; "resolution from docs"
    )]
    #[test_case(
        PNPM7,
        "apps/web",
        "next",
        "12.2.5",
        Ok(Some("12.2.5_ir3quccc6i62x6qn6jjhyjjiey"))
        ; "resolution from web"
    )]
    #[test_case(
        PNPM7,
        "apps/web",
        "typescript",
        "^4.5.3",
        Ok(Some("4.8.3"))
        ; "no peer deps"
    )]
    #[test_case(
        PNPM7,
        "apps/web",
        "lodash",
        "bad-tag",
        Ok(None)
        ; "bad tag"
    )]
    #[test_case(
        PNPM7,
        "apps/web",
        "lodash",
        "^4.17.21",
        Ok(Some("4.17.21_ehchni3mpmovsvjxesffg2i5a4"))
        ; "patched lodash"
    )]
    #[test_case(
        PNPM7,
        "apps/docs",
        "dashboard-icons",
        "github:peerigon/dashboard-icons",
        Ok(Some("github.com/peerigon/dashboard-icons/ce27ef933144e09cef3911025f3649040a8571b6"))
        ; "github dependency"
    )]
    #[test_case(
        PNPM7,
        "",
        "turbo",
        "latest",
        Ok(Some("1.4.6"))
        ; "root dependency"
    )]
    #[test_case(
        PNPM7,
        "apps/bad_workspace",
        "turbo",
        "latest",
        Err("Workspace 'apps/bad_workspace' not found in lockfile")
        ; "invalid workspace"
    )]
    #[test_case(
        PNPM8,
        "packages/a",
        "c",
        "workspace:*",
        Ok(Some("link:../c"))
        ; "v6 workspace"
    )]
    #[test_case(
        PNPM8,
        "packages/a",
        "is-odd",
        "^3.0.1",
        Ok(Some("3.0.1"))
        ; "v6 external package"
    )]
    #[test_case(
        PNPM8,
        "packages/b",
        "is-odd",
        "^3.0.1",
        Ok(None)
        ; "v6 missing"
    )]
    #[test_case(
        PNPM8,
        "apps/bad_workspace",
        "is-odd",
        "^3.0.1",
        Err("Workspace 'apps/bad_workspace' not found in lockfile")
        ; "v6 missing workspace"
    )]
    fn test_specifier_resolution(
        lockfile: &[u8],
        workspace_path: &str,
        package: &str,
        specifier: &str,
        expected: Result<Option<&str>, &str>,
    ) {
        let lockfile = PnpmLockfileData::from_bytes(lockfile).unwrap();

        let actual = lockfile.resolve_specifier(workspace_path, package, specifier);
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
            (Err(actual), Err(expected_msg)) => assert!(
                actual.to_string().contains(expected_msg),
                "Expected '{}' to appear in error message: '{}'",
                expected_msg,
                actual,
            ),
            (actual, expected) => {
                panic!("Mismatched result variants: {:?} {:?}", actual, expected)
            }
        }
    }

    #[test_case(
        PNPM7,
        "apps/docs",
        "dashboard-icons",
        "github:peerigon/dashboard-icons",
        Ok(Some(crate::Package {
            key: "github.com/peerigon/dashboard-icons/ce27ef933144e09cef3911025f3649040a8571b6".into(),
            version: "1.0.0".into(),
        }))
        ; "git package"
    )]
    #[test_case(
        PNPM_ABSOLUTE,
        "packages/a",
        "child",
        "/@scope/child/1.0.0",
        Ok(Some(crate::Package {
            key: "/@scope/child/1.0.0".into(),
            version: "1.0.0".into(),
        }))
        ; "absolute package"
    )]
    #[test_case(
        PNPM_ABSOLUTE_V6,
        "packages/a",
        "child",
        "/@scope/child@1.0.0",
        Ok(Some(crate::Package {
            key: "/@scope/child@1.0.0".into(),
            version: "1.0.0".into(),
        }))
        ; "v6 absolute package"
    )]
    #[test_case(
        PNPM_PEER,
        "apps/web",
        "next",
        "13.0.4",
        Ok(Some(crate::Package {
            key: "/next@13.0.4(react-dom@18.2.0)(react@18.2.0)".into(),
            version: "13.0.4(react-dom@18.2.0)(react@18.2.0)".into(),
        }))
        ; "v6 peer package"
    )]
    #[test_case(
        PNPM_TOP_LEVEL_OVERRIDE,
        "packages/a",
        "ci-info",
        "3.7.1",
        Ok(Some(crate::Package {
            key: "/ci-info/3.7.1".into(),
            version: "3.7.1".into(),
        }))
        ; "top level override"
    )]
    #[test_case(
        PNPM_OVERRIDE,
        "config/hardhat",
        "@nomiclabs/hardhat-ethers",
        "npm:hardhat-deploy-ethers@0.3.0-beta.13",
        Ok(Some(crate::Package {
            key: "/hardhat-deploy-ethers/0.3.0-beta.13_yab2ug5tvye2kp6e24l5x3z7uy".into(),
            version: "0.3.0-beta.13_yab2ug5tvye2kp6e24l5x3z7uy".into(),
        }))
        ; "pnpm override"
    )]
    fn test_resolve_package(
        lockfile: &[u8],
        workspace_path: &str,
        package: &str,
        specifier: &str,
        expected: Result<Option<crate::Package>, &str>,
    ) {
        let lockfile = PnpmLockfileData::from_bytes(lockfile).unwrap();
        let actual = lockfile.resolve_package(workspace_path, package, specifier);
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
            (Err(actual), Err(expected_msg)) => assert!(
                actual.to_string().contains(expected_msg),
                "Expected '{}' to appear in error message: '{}'",
                expected_msg,
                actual,
            ),
            (actual, expected) => {
                panic!("Mismatched result variants: {:?} {:?}", actual, expected)
            }
        }
    }
}
