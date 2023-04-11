mod de;
mod identifiers;
mod resolution;
mod ser;

use std::{
    collections::{HashMap, HashSet},
    iter,
    path::{Path, PathBuf},
};

use identifiers::{Descriptor, Ident, Locator};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use self::resolution::{parse_resolution, Resolution};
use super::Lockfile;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unable to parse")]
    Identifiers(#[from] identifiers::Error),
    #[error("unable to find original package in patch locator {0}")]
    PatchMissingOriginalLocator(Locator<'static>),
    #[error("unable to parse resolutions field")]
    Resolutions(#[from] resolution::Error),
    #[error("unable to find entry for {0}")]
    MissingPackageForLocator(Locator<'static>),
}

// We depend on BTree iteration being sorted
type Map<K, V> = std::collections::BTreeMap<K, V>;

pub struct BerryLockfile<'a> {
    data: &'a LockfileData,
    resolutions: Map<Descriptor<'a>, Locator<'a>>,
    locator_package: Map<Locator<'a>, &'a BerryPackage>,
    // Map of regular locators to patch locators that apply to them
    patches: Map<Locator<'static>, Locator<'a>>,
    // Descriptors that come from default package extensions that ship with berry
    extensions: HashSet<Descriptor<'static>>,
    // Package overrides
    overrides: Map<Resolution<'a>, &'a str>,
}

// This is the direct representation of the lockfile as it appears on disk.
// More internal tracking is required for effectively altering the lockfile
#[derive(Debug, Deserialize, Serialize)]
pub struct LockfileData {
    #[serde(rename = "__metadata")]
    metadata: Metadata,
    #[serde(flatten)]
    packages: Map<String, BerryPackage>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    version: u64,
    cache_key: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct BerryPackage {
    version: SemverString,
    language_name: Option<String>,
    dependencies: Option<Map<String, SemverString>>,
    peer_dependencies: Option<Map<String, SemverString>>,
    dependencies_meta: Option<Map<String, DependencyMeta>>,
    peer_dependencies_meta: Option<Map<String, DependencyMeta>>,
    // Structured metadata we need to persist
    bin: Option<Map<String, SemverString>>,
    link_type: Option<String>,
    resolution: String,
    checksum: Option<String>,
    conditions: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Clone, Copy)]
struct DependencyMeta {
    optional: Option<bool>,
    unplugged: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BerryManifest {
    resolutions: Option<Map<String, String>>,
}

impl<'a> BerryLockfile<'a> {
    pub fn new(
        lockfile: &'a LockfileData,
        manifest: Option<&'a BerryManifest>,
    ) -> Result<Self, Error> {
        let mut patches = Map::new();
        let mut locator_package = Map::new();
        let mut descriptor_locator = Map::new();
        for (key, package) in &lockfile.packages {
            let locator = Locator::try_from(package.resolution.as_str())?;

            // TODO we're ignoring buildin patches, should we not?
            if let Some(path_file) = locator.patch_file() {
                // in go code we just produce the original by replacing the ref with
                // "npm:{package.version}" I think we can extract this from the
                // locator itself
                let original_locator = locator
                    .patched_locator()
                    .ok_or_else(|| Error::PatchMissingOriginalLocator(locator.as_owned()))?;
                patches.insert(original_locator.as_owned(), locator.clone());
            }

            locator_package.insert(locator.clone(), package);

            for descriptor in Descriptor::from_lockfile_key(key) {
                let descriptor = descriptor?;
                descriptor_locator.insert(descriptor, locator.clone());
            }
        }

        let overrides = manifest
            .and_then(|manifest| manifest.resolutions())
            .transpose()?
            .unwrap_or_default();

        // A temporary representation that is keyed off of the ident to allow for faster
        // finding of possible descriptor matches
        let mut descriptor_by_indent: Map<Ident, HashSet<&str>> = Map::new();
        for descriptor in descriptor_locator.keys() {
            let ranges = descriptor_by_indent
                .entry(descriptor.ident.clone())
                .or_default();
            ranges.insert(&descriptor.range);
        }
        for package in lockfile.packages.values() {
            if let Some(deps) = &package.dependencies {
                for (name, range) in deps {
                    let ident = Ident::try_from(name.as_str())?;
                    if let Some(ranges) = descriptor_by_indent.get_mut(&ident) {
                        // If a full range contains the range of an entry then
                        // the descriptor can be accounted for.
                        // We keep any range that doesn't contain the range listed in the entry
                        ranges.retain(|full_range| !full_range.contains(range.as_ref()))
                    } // should there ever be a time where we don't have a
                      // matching ident?
                }
            }
        }

        // we go through every dep package
        // if we can't find a descriptor for a given ident then we should add a map
        // how does this work for pkg specific overrides?
        // need to get resolution field as otherwise impossible to tell which version
        // should be used

        let mut extensions = HashSet::new();
        for (ident, ranges) in descriptor_by_indent {
            for range in ranges {
                extensions.insert(Descriptor {
                    ident: ident.into_owned(),
                    range: range.to_string().into(),
                });
            }
        }

        // make sure to filter out any idents with no ranges

        // instead of generating all possible descriptors we could just check the ident
        // & that the descriptor minus the protocol

        // list of package extensions is just descriptors - any that appear to come from
        // a dependency

        // overrides essentially inject a descriptor with an exact version
        // this descriptor should be used as the default if it appears an entry's dep
        // doesn't exist e.g. lodash@npm:^4.17.20 doesn't exist
        // we should then look any lodash@ and use that instead

        // we'll need to keep a list of these mappings around for all deps
        Ok(Self {
            data: lockfile,
            resolutions: descriptor_locator,
            locator_package,
            patches,
            extensions,
            overrides,
        })
    }

    pub fn patches(&self) -> Vec<&Path> {
        self.patches
            .values()
            .filter_map(|patch| patch.patch_file())
            .filter(|path| !Locator::is_patch_builtin(path))
            .map(Path::new)
            .collect()
    }

    fn locator_to_descriptors(&self) -> HashMap<&Locator<'a>, HashSet<&Descriptor<'a>>> {
        let mut reverse_lookup: HashMap<&Locator, HashSet<&Descriptor>> =
            HashMap::with_capacity(self.locator_package.len());

        for (descriptor, locator) in &self.resolutions {
            reverse_lookup
                .entry(locator)
                .or_default()
                .insert(descriptor);
        }

        reverse_lookup
    }

    pub fn lockfile(&self) -> Result<LockfileData, Error> {
        let mut packages: std::collections::BTreeMap<String, BerryPackage> = Map::new();
        let mut metadata = self.data.metadata.clone();
        let reverse_lookup = self.locator_to_descriptors();

        for (locator, descriptors) in reverse_lookup {
            let mut descriptors = descriptors
                .into_iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>();
            descriptors.sort();
            let key = descriptors.join(", ");

            let package = self
                .locator_package
                .get(locator)
                .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))?;
            packages.insert(key, (*package).clone());
        }

        // If there aren't any checksums in the lockfile, then cache key is omitted
        if self
            .resolutions
            .values()
            .map(|locator| {
                self.locator_package
                    .get(locator)
                    .unwrap_or_else(|| panic!("No entry found for {locator}"))
            })
            .all(|pkg| pkg.checksum.is_none())
        {
            metadata.cache_key = None;
        }

        Ok(LockfileData { metadata, packages })
    }

    pub fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<BerryLockfile<'a>, Error> {
        let reverse_lookup = self.locator_to_descriptors();

        let mut resolutions = Map::new();
        let mut patches = Map::new();

        // Include all workspace packages and their references
        for (locator, package) in &self.locator_package {
            if workspace_packages
                .iter()
                .map(|s| s.as_str())
                .chain(iter::once("."))
                .any(|path| locator.is_workspace_path(path))
            {
                //  we need to all descriptors coming out the workspace
                for (name, range) in package.dependencies.iter().flatten() {
                    let dependency = self.resolve_dependency(locator, name, range.as_ref())?;
                    // we add this dependency to the resolutions
                    let dep_locator = self
                        .resolutions
                        .get(&dependency)
                        .unwrap_or_else(|| panic!("No locator found for {dependency}"));
                    resolutions.insert(dependency, dep_locator.clone());
                }

                if let Some(descriptors) = reverse_lookup.get(locator) {
                    for descriptor in descriptors {
                        resolutions.insert((*descriptor).clone(), locator.clone());
                    }
                }
            }
        }

        for key in packages {
            let locator = Locator::try_from(key.as_str())?;

            let package = self
                .locator_package
                .get(&locator)
                .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))?;

            for (name, range) in package.dependencies.iter().flatten() {
                let dependency = self.resolve_dependency(&locator, &name, range.as_ref())?;
                let dep_locator = self.resolutions.get(&dependency).unwrap();
                resolutions.insert(dependency, dep_locator.clone());
            }

            // these packages are included, we must figure out which descriptors are
            // included we just lookup the package and calculate all of the
            // descriptors it creates
            if let Some(patch_locator) = self.patches.get(&locator) {
                patches.insert(locator.as_owned(), patch_locator.clone());
                let patch_descriptors = reverse_lookup
                    .get(patch_locator)
                    .unwrap_or_else(|| panic!("No descriptors found for {patch_locator}"));
                for patch_descriptor in patch_descriptors {
                    resolutions.insert((*patch_descriptor).clone(), patch_locator.clone());
                }
            }
        }

        for (primary, patch) in &self.patches {
            let primary_descriptors = reverse_lookup.get(primary).unwrap();
            let patch_descriptors = reverse_lookup.get(patch).unwrap();

            // For each patch descriptor we extract the primary descriptor that each patch
            // descriptor targets and check if that descriptor is present in the
            // pruned map and add it if it is present
            for patch_descriptor in patch_descriptors {
                let version = patch_descriptor.primary_version().unwrap();
                let primary_descriptor = Descriptor {
                    ident: patch_descriptor.ident.clone(),
                    range: version.into(),
                };

                if resolutions.contains_key(&primary_descriptor) {
                    resolutions.insert((*patch_descriptor).clone(), patch.clone());
                }
            }
        }

        for descriptor in &self.extensions {
            // TODO graceful
            let locator = self.resolutions.get(descriptor).unwrap();
            resolutions.insert(descriptor.clone(), locator.clone());
        }

        Ok(Self {
            data: self.data,
            resolutions,
            // We rely on resolutions only containing the required locators
            // for proper pruning.
            locator_package: self.locator_package.clone(),
            patches,
            extensions: self.extensions.clone(),
            overrides: self.overrides.clone(),
        })
    }

    fn resolve_dependency(
        &self,
        locator: &Locator,
        name: &'a str,
        range: &'a str,
    ) -> Result<Descriptor<'a>, Error> {
        let mut dependency = Descriptor::new(name, range)?;

        for (resolution, reference) in &self.overrides {
            if let Some(override_dependency) =
                resolution.reduce_dependency(reference, &dependency, locator)
            {
                dependency = override_dependency;
            }
        }

        Ok(dependency)
    }
}

impl<'a> Lockfile for BerryLockfile<'a> {
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        // Retrieving the workspace package is necessary in case there's a
        // workspace specific override.
        // In practice, this is extremely silly since changing the version of
        // the dependency in the workspace's package.json does the same thing.
        let workspace_locator = self
            .locator_package
            .keys()
            .find(|locator| {
                locator.reference.starts_with("workspace:")
                    && locator.reference.ends_with(workspace_path)
            })
            .ok_or_else(|| crate::Error::MissingWorkspace(workspace_path.to_string()))?;

        let mut dependency = Descriptor::new(name, version)
            .unwrap_or_else(|_| panic!("{name} is an invalid lockfile identifier"));
        for (resolution, reference) in &self.overrides {
            if let Some(override_dependency) =
                resolution.reduce_dependency(reference, &dependency, workspace_locator)
            {
                dependency = override_dependency;
            }
        }

        let locator = self
            .resolutions
            .get(&dependency)
            .ok_or_else(|| crate::Error::MissingPackage(dependency.to_string()))?;

        let package = self
            .locator_package
            .get(locator)
            .ok_or_else(|| crate::Error::MissingPackage(dependency.to_string()))?;

        Ok(Some(crate::Package {
            key: locator.to_string(),
            version: package.version.clone().into(),
        }))
    }

    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        let locator =
            Locator::try_from(key).unwrap_or_else(|_| panic!("Was passed invalid locator: {key}"));
        let package = self.locator_package.get(&locator);

        if package.is_none() {
            return Ok(None);
        }

        let package = package.unwrap();

        let mut map = HashMap::new();
        for (name, version) in package.dependencies.iter().flatten() {
            let mut dependency = Descriptor::new(name, version.as_ref()).unwrap();
            for (resolution, reference) in &self.overrides {
                if let Some(override_dependency) =
                    resolution.reduce_dependency(reference, &dependency, &locator)
                {
                    dependency = override_dependency;
                    break;
                }
            }
            map.insert(dependency.ident.to_string(), dependency.range.to_string());
        }
        // For each dependency we need to check if there's an override
        Ok(Some(map))
    }
}

impl BerryManifest {
    pub fn resolutions(&self) -> Option<Result<Map<Resolution, &str>, Error>> {
        self.resolutions.as_ref().map(|resolutions| {
            resolutions
                .iter()
                .map(|(resolution, reference)| {
                    let res = parse_resolution(resolution)?;
                    Ok((res, reference.as_str()))
                })
                .collect()
        })
    }
}

// Newtype used exclusively for correct deserialization
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Default, Clone)]
struct SemverString(String);

impl From<SemverString> for String {
    fn from(value: SemverString) -> Self {
        value.0
    }
}

impl AsRef<str> for SemverString {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_deserialize_lockfile() {
        let lockfile: LockfileData =
            serde_yaml::from_slice(include_bytes!("../../fixtures/berry.lock")).unwrap();
        assert_eq!(lockfile.metadata.version, 6);
        assert_eq!(lockfile.metadata.cache_key.as_deref(), Some("8c0"));
    }

    #[test]
    fn test_roundtrip() {
        let contents = include_str!("../../fixtures/berry.lock");
        let lockfile: LockfileData = serde_yaml::from_str(contents).unwrap();
        let new_contents = lockfile.to_string();
        assert_eq!(contents, new_contents);
    }

    #[test]
    fn test_basic_descriptor_prune() {
        let data: LockfileData =
            serde_yaml::from_str(include_str!("../../fixtures/minimal-berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(&data, None).unwrap();

        let pruned_lockfile = lockfile
            .subgraph(
                &["packages/a".into(), "packages/c".into()],
                &["lodash@npm:4.17.21".into()],
            )
            .unwrap();

        let lodash_desc = pruned_lockfile
            .resolutions
            .get(&Descriptor::new("lodash", "npm:^4.17.0").unwrap());
        assert!(lodash_desc.is_some());
        assert_eq!(lodash_desc.unwrap().reference, "npm:4.17.21");
    }
}
