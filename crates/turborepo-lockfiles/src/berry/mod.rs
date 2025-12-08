mod de;
mod identifiers;
mod protocol_resolver;
mod resolution;
mod ser;

use std::{
    any::Any,
    borrow::Cow,
    collections::{HashMap, HashSet},
    iter,
    sync::Arc,
};

use de::Entry;
use identifiers::{Descriptor, Ident, Locator};
use protocol_resolver::DescriptorResolver;
use serde::Deserialize;
use thiserror::Error;
use turbopath::RelativeUnixPathBuf;

use self::resolution::{Resolution, parse_resolution};
use super::Lockfile;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unable to parse yaml: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("Unable to parse json: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("Unable to parse identifier: {0}")]
    Identifiers(#[from] identifiers::Error),
    #[error("Unable to find original package in patch locator {0}")]
    PatchMissingOriginalLocator(Locator<'static>),
    #[error("Unable to parse resolutions field: {0}")]
    Resolutions(#[from] resolution::Error),
    #[error("Unable to find entry for {0}")]
    MissingPackageForLocator(Locator<'static>),
    #[error("Unable to find any locator for {0}")]
    MissingLocator(Descriptor<'static>),
    #[error(
        "Unable to find catalog definition for {package} in catalog '{catalog}'. Ensure your \
         catalogs are defined in .yarnrc.yml or package.json"
    )]
    MissingCatalog { package: String, catalog: String },
    #[error("Descriptor collision {descriptor} and {other}")]
    DescriptorCollision {
        descriptor: Descriptor<'static>,
        other: String,
    },
    #[error("Unable to parse as patch reference: {0}")]
    InvalidPatchReference(String),
}

// We depend on BTree iteration being sorted for correct serialization
type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug)]
pub struct BerryLockfile {
    data: LockfileData,
    resolutions: Map<Descriptor<'static>, Locator<'static>>,
    // A mapping from descriptors without protocols to a range with a protocol
    resolver: DescriptorResolver,
    locator_package: Map<Locator<'static>, BerryPackage>,
    // Map of regular locators to patch locators that apply to them
    patches: Map<Locator<'static>, Locator<'static>>,
    // Descriptors that come from default package extensions that ship with berry
    extensions: HashSet<Descriptor<'static>>,
    // Package overrides
    overrides: Map<Resolution, String>,
    // Map from workspace paths to package locators
    workspace_path_to_locator: HashMap<String, Locator<'static>>,
    // Default catalog mappings (package name -> version)
    catalog: Arc<Map<String, String>>,
    // Named catalog mappings (catalog name -> package name -> version)
    catalogs: Arc<Map<String, Map<String, String>>>,
}

// This is the direct representation of the lockfile as it appears on disk.
// More internal tracking is required for effectively altering the lockfile
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "Map<String, Entry>")]
pub struct LockfileData {
    metadata: Metadata,
    packages: Map<String, BerryPackage>,
}

#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Clone)]
struct Metadata {
    version: String,
    cache_key: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Default, Clone)]
struct BerryPackage {
    version: String,
    language_name: Option<String>,
    dependencies: Option<Map<String, String>>,
    peer_dependencies: Option<Map<String, String>>,
    dependencies_meta: Option<Map<String, DependencyMeta>>,
    peer_dependencies_meta: Option<Map<String, DependencyMeta>>,
    // Structured metadata we need to persist
    bin: Option<Map<String, String>>,
    link_type: Option<String>,
    resolution: String,
    checksum: Option<String>,
    conditions: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord, Clone, Copy)]
struct DependencyMeta {
    optional: Option<bool>,
    unplugged: Option<bool>,
    built: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BerryManifest {
    resolutions: Option<Map<String, String>>,
    catalog: Option<Map<String, String>>,
    catalogs: Option<Map<String, Map<String, String>>>,
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
        let mut workspace_path_to_locator = HashMap::new();
        for (key, package) in &lockfile.packages {
            let locator = Locator::try_from(package.resolution.as_str())?;

            if locator.patch_file().is_some() {
                let original_locator = locator
                    .patched_locator()
                    .ok_or_else(|| Error::PatchMissingOriginalLocator(locator.as_owned()))?;
                patches.insert(original_locator.as_owned(), locator.as_owned());
            }

            locator_package.insert(locator.as_owned(), package.clone());

            if let Some(path) = locator.reference.strip_prefix("workspace:") {
                workspace_path_to_locator.insert(path.to_string(), locator.as_owned());
            }

            for descriptor in Descriptor::from_lockfile_key(key) {
                let descriptor = descriptor?;
                if let Some(other) = resolver.insert(&descriptor) {
                    Err(Error::DescriptorCollision {
                        descriptor: descriptor.clone().into_owned(),
                        other,
                    })?;
                }
                descriptor_locator.insert(descriptor.into_owned(), locator.as_owned());
            }
        }

        let (overrides, catalog, catalogs) = if let Some(manifest) = manifest {
            let catalog = Arc::new(manifest.catalog.unwrap_or_default());
            let catalogs = Arc::new(manifest.catalogs.unwrap_or_default());
            let overrides = manifest
                .resolutions
                .map(|resolutions| {
                    resolutions
                        .into_iter()
                        .map(|(resolution, reference)| {
                            let res = parse_resolution(&resolution)?;
                            Ok::<(Resolution, String), Error>((res, reference))
                        })
                        .collect()
                })
                .transpose()?
                .unwrap_or_default();
            (overrides, catalog, catalogs)
        } else {
            (Map::new(), Arc::new(Map::new()), Arc::new(Map::new()))
        };

        let mut this = Self {
            data: lockfile,
            resolutions: descriptor_locator,
            locator_package,
            resolver,
            patches,
            overrides,
            extensions: Default::default(),
            workspace_path_to_locator,
            catalog,
            catalogs,
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
                // Skip catalog references that can't be resolved during extension detection.
                // These aren't package extensions anyway, and the error will be surfaced
                // later when the catalog reference is actually used (e.g., in resolve_package).
                let Ok(mut descriptor) = self.resolve_dependency(locator, name, range.as_ref())
                else {
                    continue;
                };
                if descriptor.protocol().is_none() {
                    if let Some(range) = self.resolver.get(&descriptor) {
                        descriptor.range = range.into();
                    }
                }
                possible_extensions.remove(&descriptor);
            }

            // For Yarn 4, remove any patch sources that are accounted for by a patch
            if let Some(Locator { ident, reference }) = locator.patched_locator() {
                possible_extensions.remove(&Descriptor {
                    ident,
                    range: reference,
                });
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
        let mut no_checksum = true;
        for pkg in self.resolutions.values().map(|locator| {
            self.locator_package
                .get(locator)
                .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))
        }) {
            let pkg = pkg?;
            no_checksum = pkg.checksum.is_none();
            if !no_checksum {
                break;
            }
        }
        if no_checksum {
            metadata.cache_key = None;
        }

        Ok(LockfileData { metadata, packages })
    }

    /// Produces a new lockfile containing only the given workspaces and
    /// packages
    fn subgraph(
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
                    let dependency = self.resolve_dependency(locator, name, range)?;
                    let dep_locator = self
                        .resolutions
                        .get(&dependency)
                        .ok_or_else(|| Error::MissingLocator(dependency.clone().into_owned()))?;
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
                let dependency = self.resolve_dependency(&locator, name, range)?;
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

            // Yarn 4 allows workspaces to depend directly on patched dependencies instead
            // of using resolutions. This results in the patched dependency appearing in the
            // closure instead of the original.
            if locator.patch_file().is_some() {
                if let Some((original, _)) =
                    self.patches.iter().find(|(_, patch)| patch == &&locator)
                {
                    patches.insert(original.as_owned(), locator.as_owned());
                    // We include the patched dependency resolution
                    let Locator { ident, reference } = original.as_owned();
                    resolutions.insert(
                        Descriptor {
                            ident,
                            range: reference,
                        },
                        original.as_owned(),
                    );
                }
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
            workspace_path_to_locator: self.workspace_path_to_locator.clone(),
            catalog: Arc::clone(&self.catalog),
            catalogs: Arc::clone(&self.catalogs),
        })
    }

    /// Resolves a catalog reference to the actual version
    /// Supports both default catalog ("catalog:") and named catalogs
    /// ("catalog:name:")
    fn resolve_catalog_version(&self, name: &str, catalog_ref: &str) -> Option<&str> {
        if let Some(stripped) = catalog_ref.strip_prefix("catalog:") {
            if stripped.is_empty() {
                // Default catalog reference: "catalog:"
                self.catalog.get(name).map(|s| s.as_str())
            } else {
                // Named catalog reference: "catalog:name:" - strip trailing colon if present
                let catalog_name = stripped.strip_suffix(':').unwrap_or(stripped);
                self.catalogs
                    .get(catalog_name)
                    .and_then(|catalog| catalog.get(name).map(|s| s.as_str()))
            }
        } else {
            None
        }
    }

    /// Extracts the catalog name from a catalog reference string.
    /// Returns "default" for the default catalog ("catalog:") and
    /// the catalog name for named catalogs ("catalog:name:" or "catalog:name").
    fn catalog_name_from_ref(catalog_ref: &str) -> &str {
        catalog_ref
            .strip_prefix("catalog:")
            .map(|s| {
                let stripped = s.strip_suffix(':').unwrap_or(s);
                if stripped.is_empty() {
                    "default"
                } else {
                    stripped
                }
            })
            .unwrap_or("default")
    }

    fn resolve_dependency(
        &self,
        locator: &Locator,
        name: &str,
        range: &str,
    ) -> Result<Descriptor<'static>, Error> {
        // Check if this is a catalog reference and resolve it
        let resolved_range: Cow<'_, str> = if range.starts_with("catalog:") {
            let resolved =
                self.resolve_catalog_version(name, range)
                    .ok_or_else(|| Error::MissingCatalog {
                        package: name.to_string(),
                        catalog: Self::catalog_name_from_ref(range).to_string(),
                    })?;
            Cow::Borrowed(resolved)
        } else {
            Cow::Borrowed(range)
        };

        let mut dependency = Descriptor::new(name, &resolved_range)?;
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

    fn locator_for_workspace_path(&self, workspace_path: &str) -> Option<&Locator<'_>> {
        self.workspace_path_to_locator
            .get(workspace_path)
            .or_else(|| {
                // This is an inefficient fallback we use in case our old logic was catching
                // edge cases that the eager approach misses.
                self.locator_package.keys().find(|locator| {
                    locator.reference.starts_with("workspace:")
                        && locator.reference.ends_with(workspace_path)
                })
            })
    }
}

impl Lockfile for BerryLockfile {
    #[tracing::instrument(skip(self))]
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
            .locator_for_workspace_path(workspace_path)
            .ok_or_else(|| crate::Error::MissingWorkspace(workspace_path.to_string()))?;

        let dependency = self.resolve_dependency(workspace_locator, name, version)?;

        let Some(locator) = self.resolutions.get(&dependency) else {
            // If resolve_dependency succeeded but we can't find a locator, the package
            // isn't in the lockfile (not a catalog issue - that would have errored above)
            return Ok(None);
        };

        let package = self
            .locator_package
            .get(locator)
            .ok_or_else(|| crate::Error::MissingPackage(dependency.to_string()))?;

        Ok(Some(crate::Package {
            key: locator.to_string(),
            version: package.version.clone(),
        }))
    }

    #[tracing::instrument(skip(self))]
    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        let locator = Locator::try_from(key).map_err(Error::from)?;

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
        let subgraph = self.subgraph(workspace_packages, packages)?;
        Ok(Box::new(subgraph))
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

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        if let Some(other) = any_other.downcast_ref::<Self>() {
            self.data.metadata.version != other.data.metadata.version
                || self.data.metadata.cache_key != other.data.metadata.cache_key
        } else {
            true
        }
    }

    fn turbo_version(&self) -> Option<String> {
        let turbo_ident = Ident::try_from("turbo").expect("'turbo' is valid identifier");
        let key = self
            .locator_package
            .keys()
            .find(|key| turbo_ident == key.ident)?;
        let entry = self.locator_package.get(key)?;
        Some(entry.version.clone())
    }

    fn human_name(&self, package: &crate::Package) -> Option<String> {
        let locator = Locator::try_from(package.key.as_str()).ok()?;
        let berry_package = self.locator_package.get(&locator)?;
        let name = locator.ident.to_string();
        let version = &berry_package.version;
        Some(format!("{name}@{version}"))
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
        Self {
            resolutions,
            catalog: None,
            catalogs: None,
        }
    }

    pub fn with_resolutions_and_catalogs<I, J, K>(
        resolutions: Option<I>,
        catalog: Option<J>,
        catalogs: Option<K>,
    ) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
        J: IntoIterator<Item = (String, String)>,
        K: IntoIterator<Item = (String, Map<String, String>)>,
    {
        Self {
            resolutions: resolutions.map(|r| r.into_iter().collect()),
            catalog: catalog.map(|c| c.into_iter().collect()),
            catalogs: catalogs.map(|c| c.into_iter().collect()),
        }
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

    pub fn into_catalogs(self) -> (Map<String, String>, Map<String, Map<String, String>>) {
        (
            self.catalog.unwrap_or_default(),
            self.catalogs.unwrap_or_default(),
        )
    }

    /// Load catalog definitions from package.json contents
    ///
    /// Expects a structure like:
    /// ```json
    /// {
    ///   "catalogs": {
    ///     "react18": {
    ///       "react": "^18.0.0",
    ///       "react-dom": "^18.0.0"
    ///     }
    ///   }
    /// }
    /// ```
    pub fn load_catalogs_from_package_json(contents: &[u8]) -> Result<Self, Error> {
        #[derive(Deserialize)]
        struct PackageJson {
            #[serde(default)]
            catalogs: Option<Map<String, Map<String, String>>>,
        }

        let package_json: PackageJson = serde_json::from_slice(contents)?;

        Ok(Self {
            resolutions: None,
            catalog: None,
            catalogs: package_json.catalogs,
        })
    }

    /// Load catalog definitions from .yarnrc.yml contents
    ///
    /// Expects a structure like:
    /// ```yaml
    /// catalogs:
    ///   react18:
    ///     react: ^18.0.0
    ///     react-dom: ^18.0.0
    /// ```
    pub fn load_catalogs_from_yarnrc_yml(contents: &[u8]) -> Result<Self, Error> {
        #[derive(Deserialize)]
        struct YarnrcYml {
            #[serde(default)]
            catalogs: Option<Map<String, Map<String, String>>>,
        }

        let yarnrc: YarnrcYml = serde_yaml::from_slice(contents)?;

        Ok(Self {
            resolutions: None,
            catalog: None,
            catalogs: yarnrc.catalogs,
        })
    }

    /// Merge catalog definitions from multiple sources
    ///
    /// This allows combining catalogs from package.json and .yarnrc.yml.
    /// Later entries take precedence over earlier ones.
    pub fn merge(self, other: Self) -> Self {
        let mut merged_catalogs = self.catalogs.unwrap_or_default();

        // Merge named catalogs from other
        if let Some(other_catalogs) = other.catalogs {
            for (name, catalog) in other_catalogs {
                merged_catalogs.insert(name, catalog);
            }
        }

        Self {
            resolutions: other.resolutions.or(self.resolutions),
            catalog: other.catalog.or(self.catalog),
            catalogs: if merged_catalogs.is_empty() {
                None
            } else {
                Some(merged_catalogs)
            },
        }
    }
}

pub fn berry_subgraph(
    contents: &[u8],
    workspace_packages: &[String],
    packages: &[String],
    resolutions: Option<HashMap<String, String>>,
    catalog: Option<HashMap<String, String>>,
    catalogs: Option<HashMap<String, HashMap<String, String>>>,
) -> Result<Vec<u8>, crate::Error> {
    let manifest = if resolutions.is_some() || catalog.is_some() || catalogs.is_some() {
        // Convert HashMaps to the format expected by BerryManifest
        let catalogs_converted = catalogs.map(|c| {
            c.into_iter()
                .map(|(k, v)| (k, v.into_iter().collect::<Map<String, String>>()))
                .collect::<HashMap<_, _>>()
        });
        Some(BerryManifest::with_resolutions_and_catalogs(
            resolutions,
            catalog,
            catalogs_converted,
        ))
    } else {
        None
    };
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
    use crate::{Package, transitive_closure};

    #[test]
    fn test_deserialize_lockfile() {
        let lockfile: LockfileData =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry.lock")).unwrap();
        assert_eq!(lockfile.metadata.version, "6");
        assert_eq!(lockfile.metadata.cache_key.as_deref(), Some("8c0"));
    }

    #[test]
    fn test_problematic_semver() {
        let lockfile =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry_semver.lock")).unwrap();
        assert_eq!(lockfile.metadata.version, "6");
        assert_eq!(lockfile.metadata.cache_key.as_deref(), Some("8"));
        assert_eq!(lockfile.packages.len(), 3);
        assert_eq!(
            lockfile
                .packages
                .get("file-source@npm:2")
                .and_then(|pkg| pkg.dependencies.as_ref())
                .and_then(|deps| deps.get("stream-source")),
            Some(&"0.10".to_string())
        );
        assert_eq!(
            lockfile
                .packages
                .get("foo@workspace:packages/foo")
                .and_then(|pkg| pkg.dependencies.as_ref())
                .and_then(|deps| deps.get("file-source")),
            Some(&"2".to_string())
        );
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
            &(["@babel/types@npm:^7.8.3"]
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
        assert_eq!(package.version, "2.0.0-next.4");

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
            false,
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
            catalog: None,
            catalogs: None,
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
        );
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
            catalog: None,
            catalogs: None,
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
            catalog: None,
            catalogs: None,
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

        let closure = transitive_closure(&lockfile, "packages/ui", unresolved_deps, false).unwrap();

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
    fn test_nonexistent_resolutions_dependencies() {
        let data: LockfileData =
            serde_yaml::from_str(include_str!("../../fixtures/yarn4-resolution.lock")).unwrap();
        let manifest = BerryManifest {
            resolutions: Some(
                [("react@^18.2.0".to_string(), "18.1.0".to_string())]
                    .iter()
                    .cloned()
                    .collect(),
            ),
            catalog: None,
            catalogs: None,
        };
        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        let actual = lockfile
            .resolve_package("packages/something", "react", "^18.2.0")
            .unwrap()
            .unwrap();
        let expected = Package {
            key: "react@npm:18.1.0".into(),
            version: "18.1.0".into(),
        };
        assert_eq!(actual, expected,);

        let pruned = lockfile
            .subgraph(
                &["packages/something".into()],
                &[
                    "react@npm:18.1.0".into(),
                    "loose-envify@npm:1.4.0".into(),
                    "js-tokens@npm:4.0.0".into(),
                ],
            )
            .unwrap();
        assert_eq!(
            pruned
                .resolve_package("packages/something", "react", "^18.2.0")
                .unwrap()
                .unwrap(),
            expected
        );
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

    #[test]
    fn test_yarn4_mixed_protocols() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/yarn4-mixed-protocol.lock"))
                .unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();
        // make sure that npm:* protocol still resolves to workspace dependency
        let a_deps = lockfile
            .all_dependencies("a@workspace:pkgs/a")
            .unwrap()
            .unwrap();
        assert_eq!(a_deps.len(), 1);
        let (c_desc, version) = a_deps.into_iter().next().unwrap();
        let c_pkg = lockfile
            .resolve_package("pkgs/a", &c_desc, &version)
            .unwrap()
            .unwrap();
        assert_eq!(c_pkg.key, "c@workspace:pkgs/c");
    }

    #[test]
    fn test_yarn4_patches_direct_dependency() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/yarn4-patch.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

        let is_odd_locator = lockfile
            .resolve_package(
                "packages/b",
                "is-odd",
                "patch:is-odd@npm%3A3.0.1#~/.yarn/patches/is-odd-npm-3.0.1-93c3c3f41b.patch",
            )
            .unwrap()
            .unwrap();

        let expected_key = "is-odd@patch:is-odd@npm%3A3.0.1#~/.yarn/patches/is-odd-npm-3.0.\
                            1-93c3c3f41b.patch::version=3.0.1&hash=9b90ad";

        assert_eq!(
            is_odd_locator,
            crate::Package {
                key: expected_key.into(),
                version: "3.0.1".into(),
            }
        );

        let deps = crate::transitive_closure(
            &lockfile,
            "packages/b",
            vec![(
                "is-odd".to_string(),
                "patch:is-odd@npm%3A3.0.1#~/.yarn/patches/is-odd-npm-3.0.1-93c3c3f41b.patch"
                    .to_string(),
            )]
            .into_iter()
            .collect(),
            false,
        )
        .unwrap();

        assert_eq!(
            deps,
            vec![
                crate::Package {
                    key: expected_key.into(),
                    version: "3.0.1".into()
                },
                crate::Package {
                    key: "is-number@npm:6.0.0".into(),
                    version: "6.0.0".into()
                }
            ]
            .into_iter()
            .collect()
        );

        let subgraph = lockfile
            .subgraph(
                &["packages/b".into(), "packages/c".into()],
                &[expected_key.into(), "is-number@npm:6.0.0".into()],
            )
            .unwrap();

        let sublockfile = subgraph.lockfile().unwrap();

        // Should contain both patched dependency and original
        assert!(sublockfile.packages.contains_key("is-odd@npm:3.0.1"));
        assert!(sublockfile.packages.contains_key(
            "is-odd@patch:is-odd@npm%3A3.0.1#~/.yarn/patches/is-odd-npm-3.0.1-93c3c3f41b.patch"
        ));

        let patches = vec![
            RelativeUnixPathBuf::new(".yarn/patches/is-odd-npm-3.0.1-93c3c3f41b.patch").unwrap(),
        ];
        assert_eq!(lockfile.patches().unwrap(), patches);
        assert_eq!(subgraph.patches().unwrap(), patches);
    }

    #[test]
    fn test_yarn4_patches_direct_and_indirect_dependency() {
        let data = LockfileData::from_bytes(include_bytes!(
            "../../fixtures/yarn4-direct-and-indirect.lock"
        ))
        .unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();

        let b_closure = lockfile
            .subgraph(
                &["packages/b".into()],
                &[
                    "is-even@npm:1.0.0".into(),
                    "is-odd@npm:0.1.2".into(),
                    "is-number@npm:3.0.0".into(),
                ],
            )
            .unwrap();

        assert_eq!(b_closure.patches().unwrap(), vec![]);

        let mut locators = b_closure
            .lockfile()
            .unwrap()
            .packages
            .values()
            .map(|package| package.resolution.clone())
            .collect::<Vec<_>>();
        locators.sort();
        assert_eq!(
            locators,
            vec![
                "b@workspace:packages/b".to_string(),
                "is-even@npm:1.0.0".to_string(),
                "is-number@npm:3.0.0".to_string(),
                "is-odd@npm:0.1.2".to_string(),
                "small-yarn4@workspace:.".to_string(),
            ]
        );
    }

    #[test]
    fn test_turbo_version() {
        let data = LockfileData::from_bytes(include_bytes!("../../fixtures/berry.lock")).unwrap();
        let lockfile = BerryLockfile::new(data, None).unwrap();
        assert_eq!(lockfile.turbo_version().as_deref(), Some("1.4.6"));
    }

    #[test]
    fn test_catalog_resolution() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        // Create a manifest with default catalog that maps lodash to ^4.17.21
        let mut catalog = Map::new();
        catalog.insert("lodash".to_string(), "^4.17.21".to_string());

        let manifest = BerryManifest {
            resolutions: None,
            catalog: Some(catalog),
            catalogs: None,
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Test that app-a can resolve lodash via catalog:
        let package = lockfile
            .resolve_package("apps/app-a", "lodash", "catalog:")
            .unwrap();

        assert_eq!(
            package,
            Some(Package {
                key: "lodash@npm:4.17.21".into(),
                version: "4.17.21".into()
            })
        );
    }

    #[test]
    fn test_named_catalog_resolution() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        // Create a manifest with named catalog
        let mut production_catalog = Map::new();
        production_catalog.insert("lodash".to_string(), "^4.17.21".to_string());

        let mut catalogs = Map::new();
        catalogs.insert("production".to_string(), production_catalog);

        let manifest = BerryManifest {
            resolutions: None,
            catalog: None,
            catalogs: Some(catalogs),
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Test that we can resolve lodash via catalog:production:
        let package = lockfile
            .resolve_package("apps/app-a", "lodash", "catalog:production:")
            .unwrap();

        assert_eq!(
            package,
            Some(Package {
                key: "lodash@npm:4.17.21".into(),
                version: "4.17.21".into()
            })
        );
    }

    #[test]
    fn test_catalog_subgraph() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        // Create a manifest with default catalog
        let mut catalog = Map::new();
        catalog.insert("lodash".to_string(), "^4.17.21".to_string());

        let manifest = BerryManifest {
            resolutions: None,
            catalog: Some(catalog.clone()),
            catalogs: None,
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Create a subgraph that includes app-a
        let subgraph = lockfile
            .subgraph(&["apps/app-a".into()], &["lodash@npm:4.17.21".into()])
            .unwrap();

        // Verify that the subgraph preserves catalog data
        assert_eq!(*subgraph.catalog, catalog);

        // Verify that the subgraph can still resolve catalog references
        let package = subgraph
            .resolve_package("apps/app-a", "lodash", "catalog:")
            .unwrap();

        assert_eq!(
            package,
            Some(Package {
                key: "lodash@npm:4.17.21".into(),
                version: "4.17.21".into()
            })
        );
    }

    #[test]
    fn test_catalog_without_manifest() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        // Create lockfile without manifest (no catalog data)
        let lockfile = BerryLockfile::new(data, None).unwrap();

        // When resolving catalog: without catalog data, it should error with
        // MissingCatalog
        let result = lockfile.resolve_package("apps/app-a", "lodash", "catalog:");

        assert!(result.is_err());
        match result {
            Err(crate::Error::Berry(Error::MissingCatalog { package, catalog })) => {
                assert_eq!(package, "lodash");
                assert_eq!(catalog, "default");
            }
            other => panic!("Expected MissingCatalog error, got: {:?}", other),
        }
    }

    #[test]
    fn test_resolve_catalog_version_method() {
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        // Create manifest with both default and named catalogs
        let mut default_catalog = Map::new();
        default_catalog.insert("lodash".to_string(), "^4.17.21".to_string());
        default_catalog.insert("react".to_string(), "^18.0.0".to_string());

        let mut dev_catalog = Map::new();
        dev_catalog.insert("lodash".to_string(), "^4.17.20".to_string());

        let mut catalogs = Map::new();
        catalogs.insert("dev".to_string(), dev_catalog);

        let manifest = BerryManifest {
            resolutions: None,
            catalog: Some(default_catalog),
            catalogs: Some(catalogs),
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Test default catalog resolution
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:"),
            Some("^4.17.21")
        );
        assert_eq!(
            lockfile.resolve_catalog_version("react", "catalog:"),
            Some("^18.0.0")
        );

        // Test named catalog resolution
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:dev:"),
            Some("^4.17.20")
        );

        // Test non-existent package in catalog
        assert_eq!(
            lockfile.resolve_catalog_version("nonexistent", "catalog:"),
            None
        );

        // Test non-existent named catalog
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:prod:"),
            None
        );

        // Test non-catalog reference
        assert_eq!(lockfile.resolve_catalog_version("lodash", "^4.17.21"), None);
    }

    #[test]
    fn test_catalog_without_manifest_fails() {
        // This reproduces the user's error: trying to prune without catalog data
        let contents = include_bytes!("../../fixtures/berry-catalogs.lock");
        let data = LockfileData::from_bytes(contents).unwrap();

        // Create lockfile WITHOUT catalog data (this is what causes the error)
        let lockfile = BerryLockfile::new(data, None).unwrap();

        // Try to resolve lodash via catalog: - this should fail with helpful error
        let result = lockfile.resolve_package("apps/app-a", "lodash", "catalog:");

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = format!("{}", err);
        assert!(
            err_msg.contains("catalog")
                && err_msg.contains("lodash")
                && err_msg.contains(".yarnrc.yml"),
            "Error message should mention catalog, package name, and .yarnrc.yml. Got: {}",
            err_msg
        );
    }

    #[test]
    fn test_subgraph_with_catalog_without_manifest_errors() {
        // This reproduces the ACTUAL user error: trying to call subgraph without
        // catalog data
        let contents = include_bytes!("../../fixtures/berry-catalogs.lock");
        let data = LockfileData::from_bytes(contents).unwrap();

        // Create lockfile WITHOUT catalog data
        let lockfile = BerryLockfile::new(data, None).unwrap();

        // Try to create a subgraph - this will fail because app-a depends on lodash via
        // catalog:
        let result = lockfile.subgraph(
            &["apps/app-a".to_string()],
            &["lodash@npm:4.17.21".to_string()],
        );

        // This should error with "Unable to find any locator for lodash@catalog:"
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("lodash") || err_msg.contains("catalog"));
    }

    #[test]
    fn test_catalog_pruning_with_berry_subgraph() {
        let contents = include_bytes!("../../fixtures/berry-catalogs.lock");

        // Create catalog data that would come from the manifest
        let mut catalog = HashMap::new();
        catalog.insert("lodash".to_string(), "^4.17.21".to_string());

        // Test pruning for app-a which depends on lodash via catalog:
        let pruned = berry_subgraph(
            contents,
            &["apps/app-a".to_string()],
            &["lodash@npm:4.17.21".to_string()],
            None,
            Some(catalog.clone()),
            None,
        )
        .unwrap();

        // Verify the pruned lockfile can be parsed and contains the expected packages
        let pruned_data = LockfileData::from_bytes(&pruned).unwrap();
        let manifest = BerryManifest {
            resolutions: None,
            catalog: Some(catalog.into_iter().collect()),
            catalogs: None,
        };
        let pruned_lockfile = BerryLockfile::new(pruned_data, Some(manifest)).unwrap();

        // Verify that app-a can still resolve lodash via catalog:
        let package = pruned_lockfile
            .resolve_package("apps/app-a", "lodash", "catalog:")
            .unwrap();

        assert_eq!(
            package,
            Some(Package {
                key: "lodash@npm:4.17.21".into(),
                version: "4.17.21".into()
            })
        );
    }

    #[test]
    fn test_load_catalogs_from_package_json() {
        let contents = include_bytes!("../../fixtures/package-with-catalogs.json");
        let manifest = BerryManifest::load_catalogs_from_package_json(contents).unwrap();

        // Verify named catalogs were loaded
        let (default_catalog, catalogs) = manifest.into_catalogs();
        assert!(default_catalog.is_empty(), "Should have no default catalog");

        assert_eq!(catalogs.len(), 2);

        // Check react18 catalog
        let react18 = catalogs.get("react18").unwrap();
        assert_eq!(react18.get("react"), Some(&"^18.0.0".to_string()));
        assert_eq!(react18.get("react-dom"), Some(&"^18.0.0".to_string()));

        // Check testing catalog
        let testing = catalogs.get("testing").unwrap();
        assert_eq!(testing.get("jest"), Some(&"^29.0.0".to_string()));
        assert_eq!(testing.get("vitest"), Some(&"^1.0.0".to_string()));
    }

    #[test]
    fn test_load_catalogs_from_yarnrc_yml() {
        let contents = include_bytes!("../../fixtures/yarnrc-with-catalogs.yml");
        let manifest = BerryManifest::load_catalogs_from_yarnrc_yml(contents).unwrap();

        // Verify named catalogs were loaded
        let (default_catalog, catalogs) = manifest.into_catalogs();
        assert!(default_catalog.is_empty(), "Should have no default catalog");

        assert_eq!(catalogs.len(), 2);

        // Check react18 catalog
        let react18 = catalogs.get("react18").unwrap();
        assert_eq!(react18.get("react"), Some(&"^18.0.0".to_string()));
        assert_eq!(react18.get("react-dom"), Some(&"^18.0.0".to_string()));

        // Check nextjs catalog
        let nextjs = catalogs.get("nextjs").unwrap();
        assert_eq!(nextjs.get("next"), Some(&"^14.0.0".to_string()));
    }

    #[test]
    fn test_merge_catalogs() {
        let contents1 = include_bytes!("../../fixtures/package-with-catalogs.json");
        let contents2 = include_bytes!("../../fixtures/yarnrc-with-catalogs.yml");

        let manifest1 = BerryManifest::load_catalogs_from_package_json(contents1).unwrap();
        let manifest2 = BerryManifest::load_catalogs_from_yarnrc_yml(contents2).unwrap();

        // Merge the two manifests
        let merged = manifest1.merge(manifest2);

        let (_, catalogs) = merged.into_catalogs();

        // Should have catalogs from both sources
        assert_eq!(catalogs.len(), 3);
        assert!(catalogs.contains_key("react18"));
        assert!(catalogs.contains_key("testing"));
        assert!(catalogs.contains_key("nextjs"));
    }

    #[test]
    fn test_catalog_loading_with_lockfile() {
        // Load catalog definitions from package.json
        let catalog_contents = include_bytes!("../../fixtures/package-with-catalogs.json");
        let mut manifest =
            BerryManifest::load_catalogs_from_package_json(catalog_contents).unwrap();

        // Manually add a default catalog for lodash (since it's not in a named catalog)
        let mut default_catalog = Map::new();
        default_catalog.insert("lodash".to_string(), "^4.17.21".to_string());
        manifest.catalog = Some(default_catalog);

        // Load the lockfile with catalog data
        let lockfile_contents = include_bytes!("../../fixtures/berry-catalogs.lock");
        let data = LockfileData::from_bytes(lockfile_contents).unwrap();
        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Test resolution with default catalog
        let package = lockfile
            .resolve_package("apps/app-a", "lodash", "catalog:")
            .unwrap();

        assert_eq!(
            package,
            Some(Package {
                key: "lodash@npm:4.17.21".into(),
                version: "4.17.21".into()
            })
        );
    }

    #[test]
    fn test_catalog_without_trailing_colon() {
        // Test that "catalog:production" (without trailing colon) works the same as
        // "catalog:production:"
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        let mut production_catalog = Map::new();
        production_catalog.insert("lodash".to_string(), "^4.17.21".to_string());

        let mut catalogs = Map::new();
        catalogs.insert("production".to_string(), production_catalog);

        let manifest = BerryManifest {
            resolutions: None,
            catalog: None,
            catalogs: Some(catalogs),
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Without trailing colon - should still resolve
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:production"),
            Some("^4.17.21")
        );

        // With trailing colon
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:production:"),
            Some("^4.17.21")
        );

        // Both should resolve to the same package
        let package_no_colon = lockfile
            .resolve_package("apps/app-a", "lodash", "catalog:production")
            .unwrap();
        let package_with_colon = lockfile
            .resolve_package("apps/app-a", "lodash", "catalog:production:")
            .unwrap();

        assert_eq!(package_no_colon, package_with_colon);
        assert_eq!(
            package_no_colon,
            Some(Package {
                key: "lodash@npm:4.17.21".into(),
                version: "4.17.21".into()
            })
        );
    }

    #[test]
    fn test_empty_default_catalog() {
        // Test resolution when default catalog exists but is empty
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        let manifest = BerryManifest {
            resolutions: None,
            catalog: Some(Map::new()), // Empty catalog
            catalogs: None,
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Should return None when package not in empty catalog
        assert_eq!(lockfile.resolve_catalog_version("lodash", "catalog:"), None);

        // Resolving should fail with MissingCatalog error since lodash is not in
        // catalog
        let result = lockfile.resolve_package("apps/app-a", "lodash", "catalog:");
        assert!(result.is_err());
        match result {
            Err(crate::Error::Berry(Error::MissingCatalog { package, catalog })) => {
                assert_eq!(package, "lodash");
                assert_eq!(catalog, "default");
            }
            other => panic!("Expected MissingCatalog error, got: {:?}", other),
        }
    }

    #[test]
    fn test_scoped_package_catalog_resolution() {
        // Test that scoped packages like @types/react work with catalogs
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        let mut catalog = Map::new();
        catalog.insert("@types/lodash".to_string(), "^4.14.0".to_string());
        catalog.insert("lodash".to_string(), "^4.17.21".to_string());

        let manifest = BerryManifest {
            resolutions: None,
            catalog: Some(catalog),
            catalogs: None,
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Scoped package should resolve from catalog
        assert_eq!(
            lockfile.resolve_catalog_version("@types/lodash", "catalog:"),
            Some("^4.14.0")
        );

        // Non-scoped package should also work
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:"),
            Some("^4.17.21")
        );
    }

    #[test]
    fn test_missing_named_catalog_error() {
        // Test that referencing a non-existent named catalog gives a helpful error
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        // Create lockfile with a "dev" catalog but not "production"
        let mut dev_catalog = Map::new();
        dev_catalog.insert("lodash".to_string(), "^4.17.20".to_string());

        let mut catalogs = Map::new();
        catalogs.insert("dev".to_string(), dev_catalog);

        let manifest = BerryManifest {
            resolutions: None,
            catalog: None,
            catalogs: Some(catalogs),
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // Try to resolve via non-existent "production" catalog
        let result = lockfile.resolve_package("apps/app-a", "lodash", "catalog:production:");

        assert!(result.is_err());
        match result {
            Err(crate::Error::Berry(Error::MissingCatalog { package, catalog })) => {
                assert_eq!(package, "lodash");
                assert_eq!(catalog, "production");
            }
            other => panic!("Expected MissingCatalog error, got: {:?}", other),
        }
    }

    #[test]
    fn test_package_not_in_named_catalog() {
        // Test error when package exists in default catalog but not in the referenced
        // named catalog
        let data =
            LockfileData::from_bytes(include_bytes!("../../fixtures/berry-catalogs.lock")).unwrap();

        let mut default_catalog = Map::new();
        default_catalog.insert("lodash".to_string(), "^4.17.21".to_string());

        let mut dev_catalog = Map::new();
        dev_catalog.insert("typescript".to_string(), "^5.0.0".to_string());

        let mut catalogs = Map::new();
        catalogs.insert("dev".to_string(), dev_catalog);

        let manifest = BerryManifest {
            resolutions: None,
            catalog: Some(default_catalog),
            catalogs: Some(catalogs),
        };

        let lockfile = BerryLockfile::new(data, Some(manifest)).unwrap();

        // lodash is in default catalog but not in dev catalog
        let result = lockfile.resolve_package("apps/app-a", "lodash", "catalog:dev:");

        assert!(result.is_err());
        match result {
            Err(crate::Error::Berry(Error::MissingCatalog { package, catalog })) => {
                assert_eq!(package, "lodash");
                assert_eq!(catalog, "dev");
            }
            other => panic!("Expected MissingCatalog error, got: {:?}", other),
        }
    }
}
