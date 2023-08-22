use std::{
    borrow::Cow,
    io::{BufWriter, Write},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use turbopath::RelativeUnixPathBuf;

use super::{dep_path::DepPath, Error, LockfileVersion};

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PnpmLockfile {
    lockfile_version: LockfileVersion,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<LockfileSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    never_built_dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    only_built_dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overrides: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package_extensions_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patched_dependencies: Option<Map<String, PatchFile>>,
    importers: Map<String, ProjectSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    packages: Option<Map<String, PackageSnapshot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<Map<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PatchFile {
    // This should be a RelativeUnixPathBuf, but since that might cause unnecessary
    // parse failures we wait until access to validate.
    path: String,
    hash: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    #[serde(flatten)]
    dependencies: DependencyInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies_meta: Option<Map<String, DependenciesMeta>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    publish_directory: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase", untagged)]
pub enum DependencyInfo {
    #[serde(rename_all = "camelCase")]
    PreV6 {
        #[serde(skip_serializing_if = "Option::is_none")]
        specifiers: Option<Map<String, String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dependencies: Option<Map<String, String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        optional_dependencies: Option<Map<String, String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dev_dependencies: Option<Map<String, String>>,
    },
    #[serde(rename_all = "camelCase")]
    V6 {
        #[serde(skip_serializing_if = "Option::is_none")]
        dependencies: Option<Map<String, Dependency>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        optional_dependencies: Option<Map<String, Dependency>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dev_dependencies: Option<Map<String, Dependency>>,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Dependency {
    specifier: String,
    version: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PackageSnapshot {
    resolution: PackageResolution,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patched: Option<bool>,

    #[serde(flatten)]
    other: Map<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct DependenciesMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    injected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PackageResolution {
    // Type field, cannot use serde(tag) due to tarball having an empty type field
    // tarball -> none
    // directory -> 'directory'
    // git repository -> 'git'
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    type_field: Option<String>,
    // Tarball fields
    #[serde(skip_serializing_if = "Option::is_none")]
    integrity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tarball: Option<String>,
    // Directory fields
    #[serde(skip_serializing_if = "Option::is_none")]
    directory: Option<String>,
    // Git repository fields
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
struct LockfileSettings {
    auto_install_peer_deps: Option<bool>,
    exclude_links_from_lockfile: Option<bool>,
}

impl PnpmLockfile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::Error> {
        let this = serde_yaml::from_slice(bytes)?;
        Ok(this)
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
        // With lockfile v6+ the lockfile version is stored as a string
        matches!(self.lockfile_version.format, super::VersionFormat::String)
    }

    fn format_key(&self, name: &str, version: &str) -> String {
        match self.is_v6() {
            true => format!("/{name}@{version}"),
            false => format!("/{name}/{version}"),
        }
    }

    // Extracts the version from a dependency path
    fn extract_version<'a>(&self, key: &'a str) -> Result<Cow<'a, str>, Error> {
        let dp = DepPath::try_from(key)?;
        // If there's a suffix, the suffix gets included as part of the version
        // so we can track patch file changes
        if let Some(suffix) = dp.peer_suffix {
            let sep = match self.is_v6() {
                true => "",
                false => "_",
            };
            Ok(format!("{}{}{}", dp.version, sep, suffix).into())
        } else {
            Ok(dp.version.into())
        }
    }

    // Returns the version override if there's an override for a package
    fn apply_overrides<'a>(&'a self, name: &str, specifier: &'a str) -> &'a str {
        self.overrides
            .as_ref()
            .and_then(|o| o.get(name))
            .map(|s| s.as_str())
            .unwrap_or(specifier)
    }

    // Given a package and version specifier resolves it to an exact version
    fn resolve_specifier<'a>(
        &'a self,
        workspace_path: &str,
        name: &str,
        specifier: &'a str,
    ) -> Result<Option<&'a str>, crate::Error> {
        let importer = self.get_workspace(workspace_path)?;

        let Some((resolved_specifier, resolved_version)) =
            importer.dependencies.find_resolution(name)
        else {
            // Check if the specifier is already an exact version
            return match self.get_packages(&self.format_key(name, specifier)) {
                Some(_) => Ok(Some(specifier)),
                None => Err(Error::MissingResolvedVersion {
                    name: name.into(),
                    specifier: specifier.into(),
                    workspace: workspace_path.into(),
                }
                .into()),
            };
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

    fn prune_patches(
        patches: &Map<String, PatchFile>,
        pruned_packages: &Map<String, PackageSnapshot>,
    ) -> Result<Map<String, PatchFile>, Error> {
        let mut pruned_patches = Map::new();
        for dependency in pruned_packages.keys() {
            let dp = DepPath::try_from(dependency.as_str())?;
            let patch_key = format!("{}@{}", dp.name, dp.version);
            if let Some(patch) = patches
                .get(&patch_key)
                .filter(|patch| dp.patch_hash() == Some(&patch.hash))
            {
                pruned_patches.insert(patch_key, patch.clone());
            }
        }
        Ok(pruned_patches)
    }
}

impl crate::Lockfile for PnpmLockfile {
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        // Check if version is a key
        if self.get_packages(version).is_some() {
            let extracted_version = self.extract_version(version)?;
            return Ok(Some(crate::Package {
                key: version.into(),
                version: extracted_version.into(),
            }));
        }

        let Some(resolved_version) = self.resolve_specifier(workspace_path, name, version)? else {
            return Ok(None);
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
            let version = pkg.version.clone().map_or_else(
                || {
                    self.extract_version(resolved_version)
                        .map(|s| s.to_string())
                },
                Ok,
            )?;
            Ok(Some(crate::Package {
                key: resolved_version.to_string(),
                version,
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
            return Ok(None);
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

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn crate::Lockfile>, crate::Error> {
        let importers = self
            .importers
            .iter()
            .filter(|(key, _)| key.as_str() == "." || workspace_packages.contains(key))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Map<_, _>>();

        let mut pruned_packages = Map::new();
        for package in packages {
            let entry = self
                .get_packages(package.as_str())
                .ok_or_else(|| crate::Error::MissingPackage(package.clone()))?;
            pruned_packages.insert(package.clone(), entry.clone());
        }
        for importer in importers.values() {
            // Find all injected packages in each workspace and include it in
            // the pruned lockfile
            for dependency in
                importer
                    .dependencies_meta
                    .iter()
                    .flatten()
                    .filter_map(|(dep, meta)| match meta.injected {
                        Some(true) => Some(dep),
                        _ => None,
                    })
            {
                let (_, version) = importer
                    .dependencies
                    .find_resolution(dependency)
                    .ok_or_else(|| Error::MissingInjectedPackage(dependency.clone()))?;

                let entry = self
                    .get_packages(version)
                    .ok_or_else(|| crate::Error::MissingPackage(version.into()))?;
                pruned_packages.insert(version.to_string(), entry.clone());
            }
        }

        let patches = self
            .patched_dependencies
            .as_ref()
            .map(|patches| Self::prune_patches(patches, &pruned_packages))
            .transpose()?;

        Ok(Box::new(Self {
            importers,
            packages: match pruned_packages.is_empty() {
                false => Some(pruned_packages),
                true => None,
            },
            lockfile_version: self.lockfile_version.clone(),
            never_built_dependencies: self.never_built_dependencies.clone(),
            only_built_dependencies: self.only_built_dependencies.clone(),
            overrides: self.overrides.clone(),
            package_extensions_checksum: self.package_extensions_checksum.clone(),
            patched_dependencies: patches,
            time: None,
            settings: self.settings.clone(),
        }))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Ok(serde_yaml::to_string(&self)?.into_bytes())
    }

    fn patches(&self) -> Result<Vec<RelativeUnixPathBuf>, crate::Error> {
        let mut patches = self
            .patched_dependencies
            .iter()
            .flatten()
            .map(|(_, patch)| RelativeUnixPathBuf::new(&patch.path))
            .collect::<Result<Vec<_>, turbopath::PathError>>()?;
        patches.sort();
        Ok(patches)
    }

    fn global_change_key(&self) -> Vec<u8> {
        let mut buf = vec![b'p', b'n', b'p', b'm', 0];

        serde_json::to_writer(
            &mut buf,
            &json!({
                "version": self.lockfile_version.version,
                "checksum": self.package_extensions_checksum,
                "overrides": self.overrides,
                "patched_deps": self.patched_dependencies,
                "settings": self.settings,
            }),
        );

        buf
    }
}

impl DependencyInfo {
    // Given a dependency will find the specifier and resolved version that
    // appear in the importer object
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

pub fn pnpm_global_change(
    prev_contents: &[u8],
    curr_contents: &[u8],
) -> Result<bool, crate::Error> {
    let prev_data = PnpmLockfile::from_bytes(prev_contents)?;
    let curr_data = PnpmLockfile::from_bytes(curr_contents)?;
    Ok(prev_data.lockfile_version != curr_data.lockfile_version
        || prev_data.package_extensions_checksum != curr_data.package_extensions_checksum
        || prev_data.overrides != curr_data.overrides
        || prev_data.patched_dependencies != curr_data.patched_dependencies
        || prev_data.settings != curr_data.settings)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use test_case::test_case;

    const PNPM6: &[u8] = include_bytes!("../../fixtures/pnpm6-workspace.yaml").as_slice();
    const PNPM7: &[u8] = include_bytes!("../../fixtures/pnpm7-workspace.yaml").as_slice();
    const PNPM8: &[u8] = include_bytes!("../../fixtures/pnpm8.yaml").as_slice();
    const PNPM8_6: &[u8] = include_bytes!("../../fixtures/pnpm-v6.1.yaml").as_slice();
    const PNPM_ABSOLUTE: &[u8] = include_bytes!("../../fixtures/pnpm-absolute.yaml").as_slice();
    const PNPM_ABSOLUTE_V6: &[u8] =
        include_bytes!("../../fixtures/pnpm-absolute-v6.yaml").as_slice();
    const PNPM_PEER: &[u8] = include_bytes!("../../fixtures/pnpm-peer-v6.yaml").as_slice();
    const PNPM_TOP_LEVEL_OVERRIDE: &[u8] =
        include_bytes!("../../fixtures/pnpm-top-level-dupe.yaml").as_slice();
    const PNPM_OVERRIDE: &[u8] = include_bytes!("../../fixtures/pnpm-override.yaml").as_slice();
    const PNPM_PATCH: &[u8] = include_bytes!("../../fixtures/pnpm-patch.yaml").as_slice();
    const PNPM_PATCH_V6: &[u8] = include_bytes!("../../fixtures/pnpm-patch-v6.yaml").as_slice();

    use super::*;
    use crate::{Lockfile, Package};

    #[test]
    fn test_roundtrip() {
        for fixture in &[PNPM6, PNPM7, PNPM8, PNPM8_6] {
            let lockfile = PnpmLockfile::from_bytes(fixture).unwrap();
            let serialized_lockfile = serde_yaml::to_string(&lockfile).unwrap();
            let lockfile_from_serialized =
                serde_yaml::from_slice(serialized_lockfile.as_bytes()).unwrap();
            assert_eq!(lockfile, lockfile_from_serialized);
        }
    }

    #[test]
    fn test_patches() {
        let lockfile =
            PnpmLockfile::from_bytes(include_bytes!("../../fixtures/pnpm-patch.yaml")).unwrap();
        assert_eq!(
            lockfile.patches().unwrap(),
            vec![
                RelativeUnixPathBuf::new("patches/@babel__core@7.20.12.patch").unwrap(),
                RelativeUnixPathBuf::new("patches/is-odd@3.0.1.patch").unwrap(),
                RelativeUnixPathBuf::new("patches/moleculer@0.14.28.patch").unwrap(),
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
        Err("Unable to find resolved version for is-odd@^3.0.1 in packages/b")
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
        let lockfile = PnpmLockfile::from_bytes(lockfile).unwrap();

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
        let lockfile = PnpmLockfile::from_bytes(lockfile).unwrap();
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

    #[test]
    fn test_prune_patches() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_PATCH).unwrap();
        let pruned = lockfile
            .subgraph(
                &["packages/dependency".into()],
                &[
                    "/is-odd/3.0.1_nrrwwz7lemethtlvvm75r5bmhq".into(),
                    "/is-number/6.0.0".into(),
                    "/@babel/core/7.20.12_3hyn7hbvzkemudbydlwjmrb65y".into(),
                    "/moleculer/0.14.28_5pk7ojv7qbqha75ozglk4y4f74_kumip57h7zlinbhp4gz3jrbqry"
                        .into(),
                ],
            )
            .unwrap();
        assert_eq!(
            pruned.patches().unwrap(),
            vec![
                RelativeUnixPathBuf::new("patches/@babel__core@7.20.12.patch").unwrap(),
                RelativeUnixPathBuf::new("patches/is-odd@3.0.1.patch").unwrap(),
                RelativeUnixPathBuf::new("patches/moleculer@0.14.28.patch").unwrap(),
            ]
        )
    }

    #[test]
    fn test_prune_patches_v6() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_PATCH_V6).unwrap();
        let pruned = lockfile
            .subgraph(
                &["packages/a".into()],
                &["/lodash@4.17.21(patch_hash=lgum37zgng4nfkynzh3cs7wdeq)".into()],
            )
            .unwrap();
        assert_eq!(
            pruned.patches().unwrap(),
            vec![RelativeUnixPathBuf::new("patches/lodash@4.17.21.patch").unwrap()]
        );

        let pruned =
            lockfile
                .subgraph(
                    &["packages/b".into()],
                    &["/@babel/helper-string-parser@7.19.\
                       4(patch_hash=wjhgmpzh47qmycrzgpeyoyh3ce)(@babel/core@7.21.0)"
                        .into()],
                )
                .unwrap();
        assert_eq!(
            pruned.patches().unwrap(),
            vec![
                RelativeUnixPathBuf::new("patches/@babel__helper-string-parser@7.19.4.patch")
                    .unwrap()
            ]
        )
    }

    #[test]
    fn test_pnpm_alias_overlap() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_ABSOLUTE).unwrap();
        let closures = crate::all_transitive_closures(
            &lockfile,
            vec![(
                "packages/a".to_string(),
                vec![
                    ("@scope/parent".to_string(), "^1.0.0".to_string()),
                    ("another".to_string(), "^1.0.0".to_string()),
                    ("special".to_string(), "npm:Special@1.2.3".to_string()),
                ]
                .into_iter()
                .collect(),
            )]
            .into_iter()
            .collect(),
        )
        .unwrap();

        let mut closure = closures
            .get("packages/a")
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        closure.sort();
        assert_eq!(
            closure,
            vec![
                Package::new("/@scope/child/1.0.0", "1.0.0"),
                Package::new("/@scope/parent/1.0.0", "1.0.0"),
                Package::new("/Special/1.2.3", "1.2.3"),
                Package::new("/another/1.0.0", "1.0.0"),
                Package::new("/foo/1.0.0", "1.0.0"),
            ],
        );
    }

    #[test]
    fn test_injected_package_round_trip() {
        let original_contents = "a:
  resolution:
    type: directory,
    directory: packages/ui,
  name: ui
  version: 0.0.0
  dev: false
b:
  resolution:
    integrity: deadbeef,
    tarball: path/to/tarball.tar.gz,
  name: tar
  version: 0.0.0
  dev: false
c:
  resolution:
    repo: great-repo.git,
    commit: greatcommit,
  name: git
  version: 0.0.0
  dev: false
";
        let original_parsed: Map<String, PackageSnapshot> =
            serde_yaml::from_str(original_contents).unwrap();
        let contents = serde_yaml::to_string(&original_parsed).unwrap();
        assert_eq!(original_contents, &contents);
    }
}
