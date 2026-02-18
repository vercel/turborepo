use std::{
    any::Any,
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};

use semver::Version;
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
    catalogs: Option<Map<String, Map<String, Dependency>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pnpmfile_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    never_built_dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    only_built_dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ignored_optional_dependencies: Option<Vec<String>>,
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
    other: Map<String, serde_yaml_ng::Value>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    inject_workspace_packages: Option<bool>,
}

impl PnpmLockfile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::Error> {
        let this = serde_yaml_ng::from_slice(bytes)?;
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
                continue;
            }

            let version_less_key = dp.name.to_string();
            if let Some(patch) = patches.get(&version_less_key) {
                pruned_patches.insert(version_less_key, patch.clone());
            }
        }
        Ok(pruned_patches)
    }

    // Create a projection of all fields in the lockfile that could affect all
    // workspaces
    fn global_fields(&self) -> GlobalFields<'_> {
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
            ignored_optional_dependencies: self.ignored_optional_dependencies.clone(),
            overrides: self.overrides.clone(),
            package_extensions_checksum: self.package_extensions_checksum.clone(),
            patched_dependencies: patches,
            snapshots: pruned_snapshots,
            time: None,
            settings: self.settings.clone(),
            pnpmfile_checksum: self.pnpmfile_checksum.clone(),
            catalogs: self.catalogs.clone(),
        }))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Ok(serde_yaml_ng::to_string(&self)?.into_bytes())
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
        // pnpm versions can include peer dependency suffixes like "1.4.6_peer_suffix"
        // or peer deps in parens like "1.4.6(react@18.2.0)".
        // Extract the base semver part for validation.
        let base_version = turbo_version
            .split(['_', '('])
            .next()
            .unwrap_or(turbo_version);
        Version::parse(base_version).ok()?;
        Some(turbo_version.to_owned())
    }

    fn human_name(&self, package: &crate::Package) -> Option<String> {
        if matches!(self.version(), SupportedLockfileVersion::V7AndV9) {
            Some(package.key.clone())
        } else {
            // TODO: this is really hacky and doesn't properly handle v5 as it uses `/` as
            // the delimiter between name and version
            Some(package.key.strip_prefix('/')?.to_owned())
        }
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
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::Lockfile;

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
            serde_yaml_ng::from_str(original_contents).unwrap();
        let contents = serde_yaml_ng::to_string(&original_parsed).unwrap();

        // serde_yml quotes strings like "0.0.0" that could be ambiguous,
        // so we verify the round-trip by re-parsing instead of comparing raw strings
        let reparsed: Map<String, PackageSnapshot> = serde_yaml_ng::from_str(&contents).unwrap();
        assert_eq!(original_parsed, reparsed);
    }

    #[test]
    fn test_turbo_version_rejects_non_semver() {
        // Malicious version strings that could be used for RCE via npx should be
        // rejected
        let malicious_versions = [
            "file:./malicious.tgz",
            "https://evil.com/malicious.tgz",
            "git+https://github.com/evil/repo.git",
            "../../../etc/passwd",
            "1.0.0 && curl evil.com",
        ];

        for malicious_version in malicious_versions {
            let yaml = format!(
                r#"lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      turbo:
        specifier: ^2.0.0
        version: {malicious_version}
"#
            );
            let lockfile = PnpmLockfile::from_bytes(yaml.as_bytes()).unwrap();
            assert_eq!(
                lockfile.turbo_version(),
                None,
                "should reject malicious version: {}",
                malicious_version
            );
        }
    }
}
