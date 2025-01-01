use std::{
    any::Any,
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};

use serde::{Deserialize, Serialize};
use turbopath::RelativeUnixPathBuf;

use super::{dep_path::DepPath, Error, LockfileVersion, SupportedLockfileVersion};

type Map<K, V> = std::collections::BTreeMap<K, V>;

type Packages = Map<String, PackageSnapshot>;
type Snapshots = Map<String, PackageSnapshotV7>;

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
    packages: Option<Packages>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshots: Option<Snapshots>,
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

    // In lockfile v7, this portion of package is stored in the top level
    // `snapshots` map as opposed to being stored inline.
    #[serde(flatten)]
    snapshot: PackageSnapshotV7,

    #[serde(skip_serializing_if = "Option::is_none")]
    patched: Option<bool>,

    #[serde(flatten)]
    other: Map<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PackageSnapshotV7 {
    #[serde(skip_serializing_if = "is_false", default)]
    optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transitive_peer_dependencies: Option<Vec<String>>,
}

fn is_false(val: &bool) -> bool {
    !val
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
    auto_install_peers: Option<bool>,
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

    fn has_package(&self, key: &str) -> bool {
        match self.version() {
            SupportedLockfileVersion::V5 | SupportedLockfileVersion::V6 => {
                self.packages.as_ref().map(|pkgs| pkgs.contains_key(key))
            }
            SupportedLockfileVersion::V7AndV9 => {
                self.snapshots.as_ref().map(|snaps| snaps.contains_key(key))
            }
        }
        .unwrap_or_default()
    }

    fn package_version(&self, key: &str) -> Option<&str> {
        let pkgs = self.packages.as_ref()?;
        let pkg = pkgs.get(key)?;
        pkg.version.as_deref()
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

    fn version(&self) -> SupportedLockfileVersion {
        if matches!(self.lockfile_version.format, super::VersionFormat::Float) {
            return SupportedLockfileVersion::V5;
        }
        match self.lockfile_version.version.as_str() {
            "7.0" | "9.0" => SupportedLockfileVersion::V7AndV9,
            _ => SupportedLockfileVersion::V6,
        }
    }

    fn format_key(&self, name: &str, version: &str) -> String {
        match self.version() {
            SupportedLockfileVersion::V5 => format!("/{name}/{version}"),
            SupportedLockfileVersion::V6 => format!("/{name}@{version}"),
            SupportedLockfileVersion::V7AndV9 => format!("{name}@{version}"),
        }
    }

    // Extracts the version from a dependency path
    fn extract_version<'a>(&self, key: &'a str) -> Result<Cow<'a, str>, Error> {
        let dp = DepPath::parse(self.version(), key)?;
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
            return Ok(self
                .has_package(&self.format_key(name, specifier))
                .then_some(specifier));
        };

        let override_specifier = self.apply_overrides(name, specifier);
        if resolved_specifier == override_specifier {
            Ok(Some(resolved_version))
        } else if self.has_package(&self.format_key(name, override_specifier)) {
            Ok(Some(override_specifier))
        } else {
            Ok(None)
        }
    }

    fn prune_patches(
        &self,
        patches: &Map<String, PatchFile>,
        pruned_packages: &Map<String, PackageSnapshot>,
    ) -> Result<Map<String, PatchFile>, Error> {
        let mut pruned_patches = Map::new();
        for dependency in pruned_packages.keys() {
            let dp = DepPath::parse(self.version(), dependency.as_str())?;
            let patch_key = format!("{}@{}", dp.name, dp.version);
            if let Some(patch) = patches.get(&patch_key).filter(|patch| {
                // In V7 patch hash isn't included in packages key, so no need to check
                matches!(self.version(), SupportedLockfileVersion::V7AndV9)
                    || dp.patch_hash() == Some(&patch.hash)
            }) {
                pruned_patches.insert(patch_key, patch.clone());
            }
        }
        Ok(pruned_patches)
    }

    // Create a projection of all fields in the lockfile that could affect all
    // workspaces
    fn global_fields(&self) -> GlobalFields {
        GlobalFields {
            version: &self.lockfile_version.version,
            checksum: self.package_extensions_checksum.as_deref(),
            overrides: self.overrides.as_ref(),
            patched_dependencies: self.patched_dependencies.as_ref(),
            settings: self.settings.as_ref(),
        }
    }

    fn pruned_packages_and_snapshots(
        &self,
        packages: &[String],
    ) -> Result<(Packages, Option<Snapshots>), crate::Error> {
        let mut pruned_packages = Map::new();
        if let Some(snapshots) = self.snapshots.as_ref() {
            let mut pruned_snapshots = Map::new();
            for package in packages {
                let entry = snapshots
                    .get(package.as_str())
                    .ok_or_else(|| crate::Error::MissingPackage(package.clone()))?;
                pruned_snapshots.insert(package.clone(), entry.clone());

                // Remove peer suffix to find the key for the package entry
                let dp = DepPath::parse(self.version(), package.as_str()).map_err(Error::from)?;
                let package_key = self.format_key(dp.name, dp.version);
                let entry = self
                    .get_packages(&package_key)
                    .ok_or_else(|| crate::Error::MissingPackage(package_key.clone()))?;
                pruned_packages.insert(package_key, entry.clone());
            }

            return Ok((pruned_packages, Some(pruned_snapshots)));
        }

        for package in packages {
            let entry = self
                .get_packages(package.as_str())
                .ok_or_else(|| crate::Error::MissingPackage(package.clone()))?;
            pruned_packages.insert(package.clone(), entry.clone());
        }
        Ok((pruned_packages, None))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GlobalFields<'a> {
    version: &'a str,
    checksum: Option<&'a str>,
    overrides: Option<&'a BTreeMap<String, String>>,
    patched_dependencies: Option<&'a BTreeMap<String, PatchFile>>,
    settings: Option<&'a LockfileSettings>,
}

impl crate::Lockfile for PnpmLockfile {
    #[tracing::instrument(skip(self))]
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        // Check if version is a key
        if self.has_package(version) {
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

        if self.has_package(&key) {
            let version = self
                .package_version(&key)
                .unwrap_or(resolved_version)
                .to_owned();
            Ok(Some(crate::Package { key, version }))
        } else if self.has_package(resolved_version) {
            let version = self.package_version(resolved_version).map_or_else(
                || {
                    self.extract_version(resolved_version)
                        .map(|s| s.to_string())
                },
                |version| Ok(version.to_string()),
            )?;
            Ok(Some(crate::Package {
                key: resolved_version.to_string(),
                version,
            }))
        } else {
            Ok(None)
        }
    }

    #[tracing::instrument(skip(self))]
    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        // Check snapshots for v7
        if let Some(snapshot) = self
            .snapshots
            .as_ref()
            .and_then(|snapshots| snapshots.get(key))
        {
            return Ok(Some(snapshot.dependencies()));
        }
        let Some(entry) = self.packages.as_ref().and_then(|pkgs| pkgs.get(key)) else {
            return Ok(None);
        };
        Ok(Some(entry.snapshot.dependencies()))
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

        let (mut pruned_packages, pruned_snapshots) =
            self.pruned_packages_and_snapshots(packages)?;
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
            .map(|patches| self.prune_patches(patches, &pruned_packages))
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
            snapshots: pruned_snapshots,
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

    fn global_change(&self, other: &dyn crate::Lockfile) -> bool {
        let any_other = other as &dyn Any;
        if let Some(other) = any_other.downcast_ref::<Self>() {
            self.global_fields() != other.global_fields()
        } else {
            true
        }
    }

    fn turbo_version(&self) -> Option<String> {
        let turbo_version = self
            .importers
            .values()
            // Look through all of the workspace packages for a turbo dependency
            // grab the first one we find.
            .find_map(|project| project.dependencies.turbo_version())?;
        Some(turbo_version.to_owned())
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

    fn turbo_version(&self) -> Option<&str> {
        let (_specifier, version) = self.find_resolution("turbo")?;
        Some(version)
    }
}

impl Dependency {
    fn as_tuple(&self) -> (&str, &str) {
        let Dependency { specifier, version } = self;
        (specifier, version)
    }
}

impl PackageSnapshotV7 {
    pub fn dependencies(&self) -> HashMap<String, String> {
        self.dependencies
            .iter()
            .flatten()
            .chain(self.optional_dependencies.iter().flatten())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
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
    use std::collections::HashSet;

    use itertools::Itertools;
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
    const PNPM_V7: &[u8] = include_bytes!("../../fixtures/pnpm-v7.yaml").as_slice();
    const PNPM_V7_PEER: &[u8] = include_bytes!("../../fixtures/pnpm-v7-peer.yaml").as_slice();
    const PNPM_V7_PATCH: &[u8] = include_bytes!("../../fixtures/pnpm-v7-patch.yaml").as_slice();
    const PNPM_V9: &[u8] = include_bytes!("../../fixtures/pnpm-v9.yaml").as_slice();
    const PNPM6_TURBO: &[u8] = include_bytes!("../../fixtures/pnpm6turbo.yaml").as_slice();
    const PNPM8_TURBO: &[u8] = include_bytes!("../../fixtures/pnpm8turbo.yaml").as_slice();

    use super::*;
    use crate::{Lockfile, Package};

    #[test_case(PNPM6)]
    #[test_case(PNPM7)]
    #[test_case(PNPM8)]
    #[test_case(PNPM8_6)]
    #[test_case(PNPM_V7)]
    #[test_case(PNPM_V7_PEER)]
    #[test_case(PNPM_V7_PATCH)]
    #[test_case(PNPM_V9)]
    fn test_roundtrip(fixture: &[u8]) {
        let lockfile = PnpmLockfile::from_bytes(fixture).unwrap();
        let serialized_lockfile = serde_yaml::to_string(&lockfile).unwrap();
        let lockfile_from_serialized =
            serde_yaml::from_slice(serialized_lockfile.as_bytes()).unwrap();
        assert_eq!(lockfile, lockfile_from_serialized);
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
    #[test_case(
        PNPM_V7,
        "packages/b",
        "is-negative",
        "https://codeload.github.com/kevva/is-negative/tar.gz/1d7e288222b53a0cab90a331f1865220ec29560c",
        Ok(Some(crate::Package {
            key: "is-negative@https://codeload.github.com/kevva/is-negative/tar.gz/1d7e288222b53a0cab90a331f1865220ec29560c".into(),
            version: "2.1.0".into(),
        }))
        ; "v7 git"
    )]
    #[test_case(
        PNPM_V7_PEER,
        "packages/a",
        "ajv-keywords",
        "^5.1.0",
        Ok(Some(crate::Package {
            key: "ajv-keywords@5.1.0(ajv@8.12.0)".into(),
            version: "5.1.0(ajv@8.12.0)".into(),
        }))
        ; "v7 peer"
    )]
    #[test_case(
        PNPM_V7_PEER,
        "packages/b",
        "ajv-keywords",
        "^5.1.0",
        Ok(Some(crate::Package {
            key: "ajv-keywords@5.1.0(ajv@8.11.0)".into(),
            version: "5.1.0(ajv@8.11.0)".into(),
        }))
        ; "v7 peer 2"
    )]
    #[test_case(
        PNPM_V9,
        "",
        "turbo",
        "canary",
        Ok(Some(crate::Package {
            key: "turbo@1.13.3-canary.1".into(),
            version: "1.13.3-canary.1".into(),
        }))
        ; "v9"
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
            false,
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

    #[test]
    fn test_missing_specifier() {
        // When comparing across git commits the `package.json` might list a
        // dependency that isn't in a previous lockfile. We must not error in
        // this case.
        let lockfile = PnpmLockfile::from_bytes(PNPM8).unwrap();
        let closures = crate::all_transitive_closures(
            &lockfile,
            vec![(
                "packages/a".to_string(),
                vec![
                    ("is-odd".to_string(), "^3.0.1".to_string()),
                    ("pad-left".to_string(), "^1.0.0".to_string()),
                ]
                .into_iter()
                .collect(),
            )]
            .into_iter()
            .collect(),
            false,
        )
        .unwrap();

        let mut a_closure = closures
            .get("packages/a")
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        a_closure.sort();

        assert_eq!(
            a_closure,
            vec![
                Package::new("/is-number@6.0.0", "6.0.0"),
                Package::new("/is-odd@3.0.1", "3.0.1"),
            ]
        )
    }

    #[test]
    fn test_settings_parsing() {
        let lockfile = PnpmLockfile::from_bytes(PNPM8_6).unwrap();
        let settings = lockfile.settings.unwrap();
        assert_eq!(settings.auto_install_peers, Some(true));
        assert_eq!(settings.exclude_links_from_lockfile, Some(false));
    }

    #[test]
    fn test_lockfile_v7_parsing() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_V7).unwrap();
        assert!(lockfile.packages.unwrap().contains_key("is-buffer@1.1.6"));
        assert!(lockfile.snapshots.unwrap().contains_key("is-buffer@1.1.6"));
    }

    #[test]
    fn test_lockfile_v7_traversal() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_V7).unwrap();
        let is_even = lockfile
            .resolve_package("packages/a", "is-even", "^1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            is_even,
            Package {
                key: "is-even@1.0.0".into(),
                version: "1.0.0".into()
            }
        );
        let is_even_deps = lockfile.all_dependencies(&is_even.key).unwrap().unwrap();
        assert_eq!(
            is_even_deps,
            vec![("is-odd".to_string(), "0.1.2".to_string())]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn test_lockfile_v7_closures() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_V7_PEER).unwrap();
        let mut workspaces = HashMap::new();
        workspaces.insert(
            "packages/a".into(),
            vec![("ajv", "^8.12.0"), ("ajv-keywords", "^5.1.0")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        );
        workspaces.insert(
            "packages/b".into(),
            vec![("ajv", "8.11.0"), ("ajv-keywords", "^5.1.0")]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        );
        let mut closures: Vec<_> = crate::all_transitive_closures(&lockfile, workspaces, false)
            .unwrap()
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().sorted().collect::<Vec<_>>()))
            .collect();
        closures.sort_by_key(|(k, _)| k.clone());
        let shared_deps: HashSet<_> = vec![
            crate::Package::new("fast-deep-equal@3.1.3", "3.1.3"),
            crate::Package::new("json-schema-traverse@1.0.0", "1.0.0"),
            crate::Package::new("punycode@2.3.1", "2.3.1"),
            crate::Package::new("require-from-string@2.0.2", "2.0.2"),
            crate::Package::new("uri-js@4.4.1", "4.4.1"),
        ]
        .into_iter()
        .collect();
        let mut expected = Vec::new();
        expected.push(("packages/a".into(), {
            let mut deps = shared_deps.clone();
            deps.insert(crate::Package::new("ajv@8.12.0", "8.12.0"));
            deps.insert(crate::Package::new(
                "ajv-keywords@5.1.0(ajv@8.12.0)",
                "5.1.0(ajv@8.12.0)",
            ));
            deps.into_iter().sorted().collect::<Vec<_>>()
        }));
        expected.push(("packages/b".into(), {
            let mut deps = shared_deps;
            deps.insert(crate::Package::new("ajv@8.11.0", "8.11.0"));
            deps.insert(crate::Package::new(
                "ajv-keywords@5.1.0(ajv@8.11.0)",
                "5.1.0(ajv@8.11.0)",
            ));
            deps.into_iter().sorted().collect::<Vec<_>>()
        }));
        assert_eq!(closures, expected);
    }

    #[test]
    fn test_lockfile_v7_subgraph() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_V7_PEER).unwrap();
        let pruned_lockfile = lockfile
            .subgraph(
                &["packages/a".into()],
                &[
                    "fast-deep-equal@3.1.3".into(),
                    "ajv@8.12.0".into(),
                    "uri-js@4.4.1".into(),
                    "punycode@2.3.1".into(),
                    "require-from-string@2.0.2".into(),
                    "ajv-keywords@5.1.0(ajv@8.12.0)".into(),
                    "json-schema-traverse@1.0.0".into(),
                ],
            )
            .unwrap() as Box<dyn Any>;

        let pruned_lockfile: &PnpmLockfile = pruned_lockfile.downcast_ref().unwrap();
        let snapshots = pruned_lockfile.snapshots.as_ref().unwrap();
        let packages = pruned_lockfile.packages.as_ref().unwrap();
        assert!(
            snapshots.contains_key("ajv-keywords@5.1.0(ajv@8.12.0)"),
            "contains snapshot with used peer dependency"
        );
        assert!(
            !snapshots.contains_key("ajv-keywords@5.1.0(ajv@8.11.0)"),
            "doesn't contains snapshot with other peer dependency"
        );
        assert!(
            packages.contains_key("ajv-keywords@5.1.0"),
            "contains shared package metadata"
        );
        assert!(
            packages.contains_key("ajv@8.12.0"),
            "contains used peer dependency"
        );
        assert!(
            snapshots.contains_key("ajv@8.12.0"),
            "contains used peer dependency"
        );
    }

    #[test]
    fn test_lockfile_v7_subgraph_patches() {
        let lockfile = PnpmLockfile::from_bytes(PNPM_V7_PATCH).unwrap();
        let pruned_lockfile = lockfile
            .subgraph(
                &["packages/a".into()],
                &[
                    "fast-deep-equal@3.1.3".into(),
                    "ajv@8.12.0".into(),
                    "uri-js@4.4.1".into(),
                    "punycode@2.3.1".into(),
                    "require-from-string@2.0.2".into(),
                    "ajv-keywords@5.1.0(patch_hash=5d3ekbiux3hfmrauqwpwb6chsq)(ajv@8.12.0)".into(),
                    "json-schema-traverse@1.0.0".into(),
                ],
            )
            .unwrap() as Box<dyn Any>;

        let pruned_lockfile: &PnpmLockfile = pruned_lockfile.downcast_ref().unwrap();
        let snapshots = pruned_lockfile.snapshots.as_ref().unwrap();
        let packages = pruned_lockfile.packages.as_ref().unwrap();
        let patches = pruned_lockfile.patched_dependencies.as_ref().unwrap();
        assert!(
            snapshots.contains_key(
                "ajv-keywords@5.1.0(patch_hash=5d3ekbiux3hfmrauqwpwb6chsq)(ajv@8.12.0)"
            ),
            "contains snapshot with used peer dependency"
        );
        assert!(
            !snapshots.contains_key(
                "ajv-keywords@5.1.0(patch_hash=5d3ekbiux3hfmrauqwpwb6chsq)(ajv@8.11.0)"
            ),
            "doesn't contains snapshot with other peer dependency"
        );
        assert!(
            snapshots.contains_key("ajv@8.12.0"),
            "contains used peer dependency"
        );
        assert!(
            packages.contains_key("ajv-keywords@5.1.0"),
            "contains shared package metadata"
        );
        assert!(
            patches.contains_key("ajv-keywords@5.1.0"),
            "contains patched dependency"
        );
    }

    #[test_case(PNPM6, None ; "v6 missing")]
    #[test_case(PNPM6_TURBO, Some("2.0.3") ; "v6")]
    #[test_case(PNPM8_TURBO, Some("2.0.3") ; "v8")]
    #[test_case(PNPM_V9, Some("1.13.3-canary.1") ; "v9")]
    fn test_turbo_version(lockfile: &[u8], expected: Option<&str>) {
        let lockfile = PnpmLockfile::from_bytes(lockfile).unwrap();
        assert_eq!(lockfile.turbo_version().as_deref(), expected);
    }
}
