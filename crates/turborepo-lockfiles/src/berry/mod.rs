mod de;
mod identifiers;
mod resolution;
mod ser;

use std::{
    collections::HashSet,
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

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    version: u64,
    cache_key: String,
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
                patches.insert(original_locator, locator.clone());
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

        // TODO don't unwrap here
        let mut dependency = Descriptor::new(name, version).unwrap();
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
    ) -> Result<Option<std::collections::HashMap<String, &str>>, crate::Error> {
        // For each dependency we need to check if there's an override
        todo!()
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
        assert_eq!(lockfile.metadata.cache_key, "8c0");
    }

    #[test]
    fn test_roundtrip() {
        let contents = include_str!("../../fixtures/berry.lock");
        let lockfile: LockfileData = serde_yaml::from_str(contents).unwrap();
        let new_contents = lockfile.to_string();
        assert_eq!(contents, new_contents);
    }
}
