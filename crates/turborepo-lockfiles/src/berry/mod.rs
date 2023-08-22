mod de;
mod identifiers;
mod protocol_resolver;
mod resolution;
mod ser;

use std::{
    collections::{HashMap, HashSet},
    io::{BufWriter, Write},
    iter,
    rc::Rc,
};

use de::SemverString;
use identifiers::{Descriptor, Locator};
use protocol_resolver::DescriptorResolver;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use turbopath::RelativeUnixPathBuf;

use self::resolution::{parse_resolution, Resolution};
use super::Lockfile;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unable to parse yaml: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("unable to parse identifier: {0}")]
    Identifiers(#[from] identifiers::Error),
    #[error("unable to find original package in patch locator {0}")]
    PatchMissingOriginalLocator(Locator<'static>),
    #[error("unable to parse resolutions field: {0}")]
    Resolutions(#[from] resolution::Error),
    #[error("unable to find entry for {0}")]
    MissingPackageForLocator(Locator<'static>),
    #[error("unable to find any locator for {0}")]
    MissingLocator(Descriptor<'static>),
}

// We depend on BTree iteration being sorted for correct serialization
type Map<K, V> = std::collections::BTreeMap<K, V>;

pub struct BerryLockfile {
    data: Rc<LockfileData>,
    resolutions: Map<Descriptor<'static>, Locator<'static>>,
    // A mapping from descriptors without protocols to a range with a protocol
    resolver: DescriptorResolver,
    locator_package: Map<Locator<'static>, Rc<BerryPackage>>,
    // Map of regular locators to patch locators that apply to them
    patches: Map<Locator<'static>, Locator<'static>>,
    // Descriptors that come from default package extensions that ship with berry
    extensions: HashSet<Descriptor<'static>>,
    // Package overrides
    overrides: Map<Resolution, String>,
}

// This is the direct representation of the lockfile as it appears on disk.
// More internal tracking is required for effectively altering the lockfile
#[derive(Debug, Deserialize, Serialize)]
pub struct LockfileData {
    #[serde(rename = "__metadata")]
    metadata: Metadata,
    #[serde(flatten)]
    packages: Map<String, Rc<BerryPackage>>,
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

impl BerryLockfile {
    pub fn load(contents: &[u8], manifest: Option<BerryManifest>) -> Result<Self, super::Error> {
        let data = LockfileData::from_bytes(contents)?;
        let lockfile = BerryLockfile::new(data, manifest)?;
        Ok(lockfile)
    }

    pub fn new(lockfile: LockfileData, manifest: Option<BerryManifest>) -> Result<Self, Error> {
        let mut patches = Map::new();
        let mut locator_package = Map::new();
        let mut descriptor_locator = Map::new();
        let mut resolver = DescriptorResolver::default();
        for (key, package) in &lockfile.packages {
            let locator = Locator::try_from(package.resolution.as_str())?;

            if locator.patch_file().is_some() {
                let original_locator = locator
                    .patched_locator()
                    .ok_or_else(|| Error::PatchMissingOriginalLocator(locator.as_owned()))?;
                patches.insert(original_locator.as_owned(), locator.as_owned());
            }

            locator_package.insert(locator.as_owned(), package.clone());

            for descriptor in Descriptor::from_lockfile_key(key) {
                let descriptor = descriptor?;
                if let Some(other) = resolver.insert(&descriptor) {
                    panic!("Descriptor collision {descriptor} and {other}");
                }
                descriptor_locator.insert(descriptor.into_owned(), locator.as_owned());
            }
        }

        let overrides = manifest
            .and_then(|manifest| manifest.resolutions())
            .transpose()?
            .unwrap_or_default();

        let mut this = Self {
            data: Rc::new(lockfile),
            resolutions: descriptor_locator,
            locator_package,
            resolver,
            patches,
            overrides,
            extensions: Default::default(),
        };

        this.populate_extensions()?;

        Ok(this)
    }

    fn populate_extensions(&mut self) -> Result<(), Error> {
        let mut possible_extensions: HashSet<_> = self
            .resolutions
            .keys()
            .filter(|descriptor| matches!(descriptor.protocol(), Some("npm")))
            .collect();
        for (locator, package) in &self.locator_package {
            for (name, range) in package.dependencies.iter().flatten() {
                let mut descriptor = self.resolve_dependency(locator, name, range.as_ref())?;
                if descriptor.protocol().is_none() {
                    if let Some(range) = self.resolver.get(&descriptor) {
                        descriptor.range = range.into();
                    }
                }
                possible_extensions.remove(&descriptor);
            }
        }

        self.extensions.extend(
            possible_extensions
                .into_iter()
                .map(|desc| desc.clone().into_owned()),
        );
        Ok(())
    }

    // Helper function for inverting the resolution map
    fn locator_to_descriptors(&self) -> HashMap<&Locator<'static>, HashSet<&Descriptor<'static>>> {
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

    /// Constructs a new lockfile data ready to be serialized
    pub fn lockfile(&self) -> Result<LockfileData, Error> {
        let mut packages = Map::new();
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
            packages.insert(key, package.clone());
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

    /// Produces a new lockfile containing only the given workspaces and
    /// packages
    pub fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<BerryLockfile, Error> {
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
                //  We need to track all of the descriptors coming out the workspace
                for (name, range) in package.dependencies.iter().flatten() {
                    let dependency = self.resolve_dependency(locator, name, range.as_ref())?;
                    let dep_locator = self
                        .resolutions
                        .get(&dependency)
                        .unwrap_or_else(|| panic!("No locator found for {dependency}"));
                    resolutions.insert(dependency, dep_locator.clone());
                }

                // Included workspaces will always have their locator listed as a descriptor.
                // All other descriptors should show up in the other workspace package
                // dependencies.
                resolutions.insert(Descriptor::from(locator.clone()), locator.clone());
            }
        }

        for key in packages {
            let locator = Locator::try_from(key.as_str())?;

            let package = self
                .locator_package
                .get(&locator)
                .cloned()
                .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))?;

            for (name, range) in package.dependencies.iter().flatten() {
                let dependency = self.resolve_dependency(&locator, name, range.as_ref())?;
                let dep_locator = self
                    .resolutions
                    .get(&dependency)
                    .ok_or_else(|| Error::MissingLocator(dependency.clone().into_owned()))?;
                resolutions.insert(dependency, dep_locator.clone());
            }

            // If the package has an associated patch we include it in the subgraph
            if let Some(patch_locator) = self.patches.get(&locator) {
                patches.insert(locator.as_owned(), patch_locator.clone());
            }
        }

        for patch in patches.values() {
            let patch_descriptors = reverse_lookup
                .get(patch)
                .unwrap_or_else(|| panic!("Unable to find {patch} in reverse lookup"));

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

        // Add any descriptors used by package extensions
        for descriptor in &self.extensions {
            let locator = self
                .resolutions
                .get(descriptor)
                .ok_or_else(|| Error::MissingLocator(descriptor.to_owned()))?;
            resolutions.insert(descriptor.clone(), locator.clone());
        }

        Ok(Self {
            data: self.data.clone(),
            resolutions,
            patches,
            // We clone the following structures without any alterations and
            // rely on resolutions being correctly pruned.
            locator_package: self.locator_package.clone(),
            resolver: self.resolver.clone(),
            extensions: self.extensions.clone(),
            overrides: self.overrides.clone(),
        })
    }

    fn resolve_dependency(
        &self,
        locator: &Locator,
        name: &str,
        range: &str,
    ) -> Result<Descriptor<'static>, Error> {
        let mut dependency = Descriptor::new(name, range)?;
        // If there's no protocol we attempt to find a known one
        if dependency.protocol().is_none() {
            if let Some(range) = self.resolver.get(&dependency) {
                dependency.range = range.to_string().into();
            }
        }

        for (resolution, reference) in &self.overrides {
            if let Some(override_dependency) =
                resolution.reduce_dependency(reference, &dependency, locator)
            {
                dependency = override_dependency;
                break;
            }
        }

        // TODO Could we dedupe and wrap in Rc?
        Ok(dependency.into_owned())
    }
}

impl Lockfile for BerryLockfile {
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

        let dependency = self
            .resolve_dependency(workspace_locator, name, version)
            .unwrap_or_else(|_| panic!("{name} is an invalid lockfile identifier"));

        let Some(locator) = self.resolutions.get(&dependency) else {
            return Ok(None);
        };

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

        let Some(package) = self.locator_package.get(&locator) else {
            return Ok(None);
        };

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

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, crate::Error> {
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
                //  We need to track all of the descriptors coming out the workspace
                for (name, range) in package.dependencies.iter().flatten() {
                    let dependency = self.resolve_dependency(locator, name, range.as_ref())?;
                    let dep_locator = self
                        .resolutions
                        .get(&dependency)
                        .unwrap_or_else(|| panic!("No locator found for {dependency}"));
                    resolutions.insert(dependency, dep_locator.clone());
                }

                // Included workspaces will always have their locator listed as a descriptor.
                // All other descriptors should show up in the other workspace package
                // dependencies.
                resolutions.insert(Descriptor::from(locator.clone()), locator.clone());
            }
        }

        for key in packages {
            // The error mapping is required to help massage the error types
            let locator = Locator::try_from(key.as_str()).map_err(Error::from)?;

            let package = self
                .locator_package
                .get(&locator)
                .cloned()
                .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))?;

            for (name, range) in package.dependencies.iter().flatten() {
                let dependency = self.resolve_dependency(&locator, name, range.as_ref())?;
                let dep_locator = self
                    .resolutions
                    .get(&dependency)
                    .ok_or_else(|| Error::MissingLocator(dependency.clone().into_owned()))?;
                resolutions.insert(dependency, dep_locator.clone());
            }

            // If the package has an associated patch we include it in the subgraph
            if let Some(patch_locator) = self.patches.get(&locator) {
                patches.insert(locator.as_owned(), patch_locator.clone());
            }
        }

        for patch in patches.values() {
            let patch_descriptors = reverse_lookup
                .get(patch)
                .unwrap_or_else(|| panic!("Unable to find {patch} in reverse lookup"));

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

        // Add any descriptors used by package extensions
        for descriptor in &self.extensions {
            let locator = self
                .resolutions
                .get(descriptor)
                .ok_or_else(|| Error::MissingLocator(descriptor.to_owned()))?;
            resolutions.insert(descriptor.clone(), locator.clone());
        }

        Ok(Box::new(Self {
            data: self.data.clone(),
            resolutions,
            patches,
            // We clone the following structures without any alterations and
            // rely on resolutions being correctly pruned.
            locator_package: self.locator_package.clone(),
            resolver: self.resolver.clone(),
            extensions: self.extensions.clone(),
            overrides: self.overrides.clone(),
        }))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Ok(self.lockfile()?.to_string().into_bytes())
    }

    fn patches(&self) -> Result<Vec<RelativeUnixPathBuf>, crate::Error> {
        let mut patches = self
            .patches
            .values()
            .filter_map(|patch| patch.patch_file())
            .filter(|path| !Locator::is_patch_builtin(path))
            .map(|s| RelativeUnixPathBuf::new(s.to_string()))
            .collect::<Result<Vec<_>, turbopath::PathError>>()?;
        patches.sort();
        Ok(patches)
    }

    fn global_change_key(&self) -> Vec<u8> {
        let mut buf = vec![b'b', b'e', b'r', b'r', b'y', 0];

        serde_json::to_writer(
            &mut buf,
            &json!({
                "version": &self.data.metadata.version,
                "cache_key": &self.data.metadata.cache_key,
            }),
        );

        buf
    }
}

impl LockfileData {
    pub fn from_bytes(s: &[u8]) -> Result<Self, Error> {
        serde_yaml::from_slice(s).map_err(Error::from)
    }
}

impl BerryManifest {
    pub fn with_resolutions<I>(resolutions: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let resolutions = Some(resolutions.into_iter().collect());
        Self { resolutions }
    }

    pub fn resolutions(self) -> Option<Result<Map<Resolution, String>, Error>> {
        self.resolutions.map(|resolutions| {
            resolutions
                .into_iter()
                .map(|(resolution, reference)| {
                    let res = parse_resolution(&resolution)?;
                    Ok((res, reference))
                })
                .collect()
        })
    }
}

pub fn berry_subgraph(
    contents: &[u8],
    workspace_packages: &[String],
    packages: &[String],
    resolutions: Option<HashMap<String, String>>,
) -> Result<Vec<u8>, crate::Error> {
    let manifest = resolutions.map(BerryManifest::with_resolutions);
    let data = LockfileData::from_bytes(contents)?;
    let lockfile = BerryLockfile::new(data, manifest)?;
    let pruned_lockfile = lockfile.subgraph(workspace_packages, packages)?;
    let new_contents = pruned_lockfile.encode()?;
    Ok(new_contents)
}

pub fn berry_global_change(prev_contents: &[u8], curr_contents: &[u8]) -> Result<bool, Error> {
    let prev_data = LockfileData::from_bytes(prev_contents)?;
    let curr_data = LockfileData::from_bytes(curr_contents)?;
    Ok(prev_data.metadata.cache_key != curr_data.metadata.cache_key
        || prev_data.metadata.version != curr_data.metadata.version)
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{transitive_closure, Package};

    #[test]
    fn test_deserialize_lockfile() {
        let lockfile: LockfileData =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry.lock")).unwrap();
        assert_eq!(lockfile.metadata.version, 6);
        assert_eq!(lockfile.metadata.cache_key.as_deref(), Some("8c0"));
    }

    #[test]
    fn test_roundtrip() {
        let contents = include_str!("../../fixtures/berry.lock");
        let lockfile = LockfileData::from_bytes(contents.as_bytes()).unwrap();
        let new_contents = lockfile.to_string();
        assert_eq!(contents, new_contents);
    }

    #[test]
    fn test_resolve_package() {
        let data: LockfileData =
            serde_yaml::from_str(include_str!("../../fixtures/berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

        assert_eq!(
            lockfile
                .resolve_package("apps/docs", "js-tokens", "^3.0.0 || ^4.0.0")
                .unwrap(),
            Some(Package {
                key: "js-tokens@npm:4.0.0".into(),
                version: "4.0.0".into()
            }),
        );
        assert_eq!(
            lockfile
                .resolve_package("apps/docs", "js-tokens", "^4.0.0")
                .unwrap(),
            Some(Package {
                key: "js-tokens@npm:4.0.0".into(),
                version: "4.0.0".into()
            }),
        );
        assert_eq!(
            lockfile
                .resolve_package("apps/docs", "eslint-config-custom", "*")
                .unwrap(),
            Some(Package {
                key: "eslint-config-custom@workspace:packages/eslint-config-custom".into(),
                version: "0.0.0-use.local".into()
            }),
        );
        assert_eq!(
            lockfile
                .resolve_package("apps/docs", "@babel/code-frame", "^7.12.11")
                .unwrap(),
            None,
        );
    }

    #[test]
    fn test_all_dependencies() {
        let data: LockfileData =
            serde_yaml::from_str(include_str!("../../fixtures/berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

        let pkg = lockfile
            .resolve_package("apps/docs", "react-dom", "18.2.0")
            .unwrap()
            .unwrap();
        let deps = lockfile.all_dependencies(&pkg.key).unwrap().unwrap();
        assert_eq!(
            deps,
            [
                ("loose-envify".to_string(), "^1.1.0".to_string()),
                ("scheduler".to_string(), "^0.23.0".to_string())
            ]
            .iter()
            .cloned()
            .collect()
        );
    }

    #[test]
    fn test_package_extension_detection() {
        let data: LockfileData =
            serde_yaml::from_str(include_str!("../../fixtures/berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

        assert_eq!(
            &lockfile.extensions,
            &(["@babel/types@npm:^7.8.3", "lodash@npm:4.17.21"]
                .iter()
                .map(|s| Descriptor::try_from(*s).unwrap())
                .collect::<HashSet<_>>())
        );
    }

    #[test]
    fn test_patch_list() {
        let data: LockfileData =
            serde_yaml::from_str(include_str!("../../fixtures/berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

        let locator = Locator::try_from("resolve@npm:2.0.0-next.4").unwrap();

        let patch = lockfile.patches.get(&locator).unwrap();
        let package = lockfile.locator_package.get(patch).unwrap();
        assert_eq!(package.version.as_ref(), "2.0.0-next.4");

        assert_eq!(
            lockfile.patches().unwrap(),
            vec![
                RelativeUnixPathBuf::new(".yarn/patches/lodash-npm-4.17.21-6382451519.patch")
                    .unwrap()
            ]
        );
    }

    #[test]
    fn test_empty_patch_list() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/minimal-berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

        let empty_vec: Vec<RelativeUnixPathBuf> = Vec::new();
        assert_eq!(lockfile.patches().unwrap(), empty_vec);
    }

    #[test]
    fn test_basic_descriptor_prune() {
        let data: LockfileData =
            serde_yaml::from_str(include_str!("../../fixtures/minimal-berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

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

        let pruned_lockfile = lockfile
            .subgraph(
                &["packages/b".into(), "packages/c".into()],
                &["lodash@npm:4.17.21".into()],
            )
            .unwrap();

        let lodash_desc = pruned_lockfile
            .resolutions
            .get(&Descriptor::new("lodash", "npm:^3.0.0 || ^4.0.0").unwrap());
        assert!(lodash_desc.is_some());
        assert_eq!(lodash_desc.unwrap().reference, "npm:4.17.21");
    }

    #[test]
    fn test_closure_with_patch() {
        let data = LockfileData::from_bytes(include_bytes!("../../fixtures/berry.lock")).unwrap();
        let resolutions = BerryManifest::with_resolutions(vec![(
            "lodash@^4.17.21".into(),
            "patch:lodash@npm%3A4.17.21#./.yarn/patches/lodash-npm-4.17.21-6382451519.patch".into(),
        )]);
        let lockfile = BerryLockfile::new(data, Some(resolutions)).unwrap();
        let closure = crate::transitive_closure(
            &lockfile,
            "apps/docs",
            HashMap::from_iter(vec![("lodash".into(), "^4.17.21".into())]),
        )
        .unwrap();

        assert!(closure.contains(&Package {
            key: "lodash@npm:4.17.21".into(),
            version: "4.17.21".into()
        }));
    }

    #[test]
    fn test_basic_resolutions_dependencies() {
        let data: LockfileData = serde_yaml::from_str(include_str!(
            "../../fixtures/minimal-berry-resolutions.lock"
        ))
        .unwrap();
        let manifest = BerryManifest {
            resolutions: Some(
                [("debug@^4.3.4".to_string(), "1.0.0".to_string())]
                    .iter()
                    .cloned()
                    .collect(),
            ),
        };
        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        let pkg = lockfile
            .resolve_package("packages/b", "debug", "^4.3.4")
            .unwrap()
            .unwrap();
        assert_eq!(
            pkg,
            Package {
                key: "debug@npm:1.0.0".into(),
                version: "1.0.0".into()
            }
        )
    }

    #[test]
    fn test_targeted_resolutions_dependencies() {
        let data: LockfileData = serde_yaml::from_str(include_str!(
            "../../fixtures/minimal-berry-resolutions.lock"
        ))
        .unwrap();
        let manifest = BerryManifest {
            resolutions: Some(
                [
                    ("debug".to_string(), "1.0.0".to_string()),
                    // This is a targeted override just for the ms dependency of the debug package
                    ("debug/ms".to_string(), "0.6.0".to_string()),
                ]
                .iter()
                .cloned()
                .collect(),
            ),
        };
        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        let deps = lockfile
            .all_dependencies("debug@npm:1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            deps,
            [("ms".to_string(), "npm:0.6.0".to_string())]
                .iter()
                .cloned()
                .collect(),
        );
        let pkg = lockfile
            .resolve_package("packages/b", "ms", "npm:0.6.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            pkg,
            Package {
                key: "ms@npm:0.6.0".into(),
                version: "0.6.0".into()
            }
        );
    }

    #[test]
    fn test_robust_resolutions_dependencies() {
        let data = LockfileData::from_bytes(include_bytes!(
            "../../fixtures/robust-berry-resolutions.lock"
        ))
        .unwrap();
        let manifest = BerryManifest {
            resolutions: Some(
                [("ajv".to_string(), "^8".to_string())]
                    .iter()
                    .cloned()
                    .collect(),
            ),
        };
        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        let unresolved_deps = vec![
            ("@types/react-dom", "^17.0.11"),
            ("@types/react", "^17.0.37"),
            ("eslint", "^7.32.0"),
            ("typescript", "^4.5.2"),
            ("react", "^18.2.0"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let closure = transitive_closure(&lockfile, "packages/ui", unresolved_deps).unwrap();

        assert!(closure.contains(&Package {
            key: "ajv@npm:8.11.2".into(),
            version: "8.11.2".into()
        }));
        assert!(closure.contains(&Package {
            key: "uri-js@npm:4.4.1".into(),
            version: "4.4.1".into()
        }));
    }

    #[test]
    fn test_workspace_collision() {
        let data = LockfileData::from_bytes(include_bytes!(
            "../../fixtures/berry-protocol-collision.lock"
        ))
        .unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();
        let no_proto = Descriptor::try_from("c@*").unwrap();
        let workspace_proto = Descriptor::try_from("c@workspace:*").unwrap();
        let full_path = Descriptor::try_from("c@workspace:packages/c").unwrap();
        let a_lockfile = lockfile
            .subgraph(&["packages/a".into(), "packages/c".into()], &[])
            .unwrap();
        let a_reverse_lookup = a_lockfile.locator_to_descriptors();
        let a_c_descriptors = a_reverse_lookup
            .get(&Locator::try_from("c@workspace:packages/c").unwrap())
            .unwrap();

        assert_eq!(
            a_c_descriptors,
            &(vec![&no_proto, &full_path]
                .into_iter()
                .collect::<HashSet<_>>())
        );

        let b_lockfile = lockfile
            .subgraph(&["packages/b".into(), "packages/c".into()], &[])
            .unwrap();
        let b_reverse_lookup = b_lockfile.locator_to_descriptors();
        let b_c_descriptors = b_reverse_lookup
            .get(&Locator::try_from("c@workspace:packages/c").unwrap())
            .unwrap();

        assert_eq!(
            b_c_descriptors,
            &(vec![&workspace_proto, &full_path]
                .into_iter()
                .collect::<HashSet<_>>())
        );
    }

    #[test]
    fn test_builtin_patch_descriptors() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-builtin.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();
        let subgraph = lockfile
            .subgraph(
                &["packages/a".into(), "packages/c".into()],
                &["resolve@npm:1.22.3".into()],
            )
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();
        let (resolve_key, _) = subgraph_data
            .packages
            .iter()
            .find(|(_, v)| {
                v.resolution
                    == "resolve@patch:resolve@npm%3A1.22.3#~builtin<compat/resolve>::version=1.22.\
                        3&hash=c3c19d"
            })
            .unwrap();
        assert_eq!(
            resolve_key,
            "resolve@patch:resolve@^1.22.0#~builtin<compat/resolve>, \
             resolve@patch:resolve@^1.22.2#~builtin<compat/resolve>"
        );
    }
}
