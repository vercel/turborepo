use std::{any::Any, collections::HashMap, str::FromStr};

use biome_json_formatter::context::JsonFormatOptions;
use biome_json_parser::JsonParserOptions;
use id::PossibleKeyIter;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use turborepo_errors::ParseDiagnostic;

use crate::Lockfile;

mod de;
mod id;

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to strip commas: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Failed to strip commas: {0}")]
    Format(#[from] biome_formatter::FormatError),
    #[error("Failed to strip commas: {0}")]
    Print(#[from] biome_formatter::PrintError),
    #[error("Turborepo cannot serialize Bun lockfiles.")]
    NotImplemented,
    #[error("{ident} had two entries with differing checksums: {sha1}, {sha2}")]
    MismatchedShas {
        ident: String,
        sha1: String,
        sha2: String,
    },
}

#[derive(Debug, Clone)]
pub struct BunLockfile {
    data: BunLockfileData,
    key_to_entry: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunLockfileData {
    #[allow(unused)]
    lockfile_version: i32,
    workspaces: Map<String, WorkspaceEntry>,
    packages: Map<String, PackageEntry>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    patched_dependencies: Map<String, String>,
}

#[derive(Debug, Deserialize, PartialEq, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dev_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_peers: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Clone)]
struct PackageEntry {
    ident: String,
    registry: Option<String>,
    // Present except for workspace & root deps
    info: Option<PackageInfo>,
    // Present on registry
    checksum: Option<String>,
    root: Option<RootInfo>,
}

#[derive(Debug, Deserialize, Default, PartialEq, Clone, Serialize)]
struct PackageInfo {
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dev_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    optional_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    peer_dependencies: Map<String, String>,
    // We do not care about the rest here
    // the values here should be generic
    #[serde(flatten)]
    other: Map<String, Value>,
}
#[derive(Debug, Deserialize, PartialEq, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RootInfo {
    bin: Option<String>,
    bin_dir: Option<String>,
}

impl Serialize for PackageEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;

        let mut tuple = serializer.serialize_tuple(4)?;
        tuple.serialize_element(&self.ident)?;

        let Some(info) = &self.info else {
            return tuple.end();
        };

        tuple.serialize_element(&self.registry.as_deref().unwrap_or(""))?;
        tuple.serialize_element(info)?;
        tuple.serialize_element(&self.checksum)?;
        if let Some(root) = &self.root {
            tuple.serialize_element(root)?;
        }
        tuple.end()
    }
}

impl Lockfile for BunLockfile {
    #[tracing::instrument(skip(self, workspace_path))]
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        let workspace_entry = self
            .data
            .workspaces
            .get(workspace_path)
            .ok_or_else(|| crate::Error::MissingWorkspace(workspace_path.into()))?;
        let workspace_name = &workspace_entry.name;
        let workspace_key = format!("{workspace_name}/{name}");
        if let Some((_key, entry)) = self.package_entry(&workspace_key) {
            let mut version = entry.version().to_string();
            // Check for any patches
            if let Some(patch) = self.data.patched_dependencies.get(&entry.ident) {
                version.push('+');
                version.push_str(patch);
            }
            // Bun's keys include how a package is imported that can result in
            // faulty cache miss if used by turbo to calculate a hash.
            // We instead use the ident (the first element of the entry) as it omits this
            // information.
            // Note: Entries are not deduplicated in `bun.lock` if
            // they need to be qualified e.g. packages a and b -> shared@1.0.0
            // and packages c and d -> shared@2.0.0 will result in one of the
            // shared entries having an unqualified key (`shared`) and the other will have
            // qualified keys of `a/shared` and `b/shared` where both will have
            // the same entry with ident of `shared@1.0.0`. Because they are
            // identical entries we do not differentiate between them even though they are
            // different entries in the map.
            Ok(Some(crate::Package {
                key: entry.ident.to_string(),
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
        let entry_key = self
            .key_to_entry
            .get(key)
            .ok_or_else(|| crate::Error::MissingPackage(key.into()))?;
        let entry = self
            .data
            .packages
            .get(entry_key)
            .ok_or_else(|| crate::Error::MissingPackage(key.into()))?;

        let mut deps = HashMap::new();
        for (dependency, version) in entry.info.iter().flat_map(|info| info.all_dependencies()) {
            let parent_key = format!("{entry_key}/{dependency}");
            let Some((dep_key, _)) = self.package_entry(&parent_key) else {
                return Err(crate::Error::MissingPackage(dependency.to_string()));
            };
            deps.insert(dep_key.to_string(), version.to_string());
        }

        Ok(Some(deps))
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
        // Ok(self.lockfile()?.to_string().into_bytes())
        // Use serde_json to encode the lockfile
        Ok(serde_json::to_vec(&self.data)?)
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        // Downcast returns none if the concrete type doesn't match
        // if the types don't match then we changed package managers
        any_other.downcast_ref::<Self>().is_none()
    }

    fn turbo_version(&self) -> Option<String> {
        let (_, entry) = self.package_entry("turbo")?;
        Some(entry.version().to_owned())
    }

    fn human_name(&self, package: &crate::Package) -> Option<String> {
        let entry = self.data.packages.get(&package.key)?;
        Some(entry.ident.clone())
    }
}

impl BunLockfile {
    pub fn from_bytes(input: &[u8]) -> Result<Self, super::Error> {
        let s = std::str::from_utf8(input).map_err(Error::from)?;
        Self::from_str(s)
    }

    // Given a specific key for a package, return the most specific key that is
    // present in the lockfile
    fn package_entry(&self, key: &str) -> Option<(&str, &PackageEntry)> {
        let (key, entry) =
            PossibleKeyIter::new(key).find_map(|k| self.data.packages.get_key_value(k))?;
        Some((key, entry))
    }

    pub fn lockfile(&self) -> Result<BunLockfileData, Error> {
        // let mut packages = Map::new();
        // let mut metadata = self.data.metadata.clone();
        // let reverse_lookup = self.locator_to_descriptors();

        // for (locator, descriptors) in reverse_lookup {
        //     let mut descriptors = descriptors
        //         .into_iter()
        //         .map(|d| d.to_string())
        //         .collect::<Vec<_>>();
        //     descriptors.sort();
        //     let key = descriptors.join(", ");

        //     let package = self
        //         .locator_package
        //         .get(locator)
        //         .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))?;
        //     packages.insert(key, package.clone());
        // }

        // // If there aren't any checksums in the lockfile, then cache key is omitted
        // let mut no_checksum = true;
        // for pkg in self.resolutions.values().map(|locator| {
        //     self.locator_package
        //         .get(locator)
        //         .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))
        // }) {
        //     let pkg = pkg?;
        //     no_checksum = pkg.checksum.is_none();
        //     if !no_checksum {
        //         break;
        //     }
        // }
        // if no_checksum {
        //     metadata.cache_key = None;
        // }

        Ok(BunLockfileData {
            lockfile_version: self.data.lockfile_version,
            workspaces: self.data.workspaces.clone(),
            packages: self.data.packages.clone(),
            patched_dependencies: self.data.patched_dependencies.clone(),
        })
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<BunLockfile, Error> {
        // let reverse_lookup = self.locator_to_descriptors();

        // let mut resolutions = Map::new();
        // let mut patches = Map::new();

        // // Include all workspace packages and their references
        // for (locator, package) in &self.locator_package {
        //     if workspace_packages
        //         .iter()
        //         .map(|s| s.as_str())
        //         .chain(iter::once("."))
        //         .any(|path| locator.is_workspace_path(path))
        //     {
        //         //  We need to track all of the descriptors coming out the workspace
        //         for (name, range) in package.dependencies.iter().flatten() {
        //             let dependency = self.resolve_dependency(locator, name,
        // range.as_ref())?;             let dep_locator = self
        //                 .resolutions
        //                 .get(&dependency)
        //                 .ok_or_else(||
        // Error::MissingLocator(dependency.clone().into_owned()))?;
        // resolutions.insert(dependency, dep_locator.clone());         }

        //         // Included workspaces will always have their locator listed as a
        // descriptor.         // All other descriptors should show up in the
        // other workspace package         // dependencies.
        //         resolutions.insert(Descriptor::from(locator.clone()),
        // locator.clone());     }
        // }

        // for key in packages {
        //     let locator = Locator::try_from(key.as_str())?;

        //     let package = self
        //         .locator_package
        //         .get(&locator)
        //         .cloned()
        //         .ok_or_else(|| Error::MissingPackageForLocator(locator.as_owned()))?;

        //     for (name, range) in package.dependencies.iter().flatten() {
        //         let dependency = self.resolve_dependency(&locator, name,
        // range.as_ref())?;         let dep_locator = self
        //             .resolutions
        //             .get(&dependency)
        //             .ok_or_else(||
        // Error::MissingLocator(dependency.clone().into_owned()))?;
        //         resolutions.insert(dependency, dep_locator.clone());
        //     }

        //     // If the package has an associated patch we include it in the subgraph
        //     if let Some(patch_locator) = self.patches.get(&locator) {
        //         patches.insert(locator.as_owned(), patch_locator.clone());
        //     }

        //     // Yarn 4 allows workspaces to depend directly on patched dependencies
        // instead     // of using resolutions. This results in the patched
        // dependency appearing in the     // closure instead of the original.
        //     if locator.patch_file().is_some() {
        //         if let Some((original, _)) =
        //             self.patches.iter().find(|(_, patch)| patch == &&locator)
        //         {
        //             patches.insert(original.as_owned(), locator.as_owned());
        //             // We include the patched dependency resolution
        //             let Locator { ident, reference } = original.as_owned();
        //             resolutions.insert(
        //                 Descriptor {
        //                     ident,
        //                     range: reference,
        //                 },
        //                 original.as_owned(),
        //             );
        //         }
        //     }
        // }

        // for patch in patches.values() {
        //     let patch_descriptors = reverse_lookup
        //         .get(patch)
        //         .unwrap_or_else(|| panic!("Unable to find {patch} in reverse
        // lookup"));

        //     // For each patch descriptor we extract the primary descriptor that each
        // patch     // descriptor targets and check if that descriptor is
        // present in the     // pruned map and add it if it is present
        //     for patch_descriptor in patch_descriptors {
        //         let version = patch_descriptor.primary_version().unwrap();
        //         let primary_descriptor = Descriptor {
        //             ident: patch_descriptor.ident.clone(),
        //             range: version.into(),
        //         };

        //         if resolutions.contains_key(&primary_descriptor) {
        //             resolutions.insert((*patch_descriptor).clone(), patch.clone());
        //         }
        //     }
        // }

        // // Add any descriptors used by package extensions
        // for descriptor in &self.extensions {
        //     let locator = self
        //         .resolutions
        //         .get(descriptor)
        //         .ok_or_else(|| Error::MissingLocator(descriptor.to_owned()))?;
        //     resolutions.insert(descriptor.clone(), locator.clone());
        // }

        Ok(Self {
            data: self.data.clone(),
            key_to_entry: self.key_to_entry.clone(),
        })
    }
}

impl FromStr for BunLockfile {
    type Err = super::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed_json = biome_json_parser::parse_json(
            s,
            JsonParserOptions::default().with_allow_trailing_commas(),
        );
        if parsed_json.has_errors() {
            let diags = parsed_json
                .into_diagnostics()
                .into_iter()
                .map(|diagnostic| ParseDiagnostic::from(&diagnostic).to_string())
                .join("\n");
            return Err(super::Error::BiomeJsonError(diags));
        }
        let syntax_tree = parsed_json.syntax();
        let format = biome_json_formatter::format_node(
            JsonFormatOptions::default()
                .with_trailing_commas(biome_json_formatter::context::TrailingCommas::None),
            &syntax_tree,
        )
        .map_err(Error::from)?;
        let strict_json = format.print().map_err(Error::from)?;
        let data: BunLockfileData = serde_json::from_str(strict_json.as_code())?;
        let mut key_to_entry = HashMap::with_capacity(data.packages.len());
        for (path, info) in data.packages.iter() {
            if let Some(prev_path) = key_to_entry.insert(info.ident.clone(), path.clone()) {
                let prev_info = data
                    .packages
                    .get(&prev_path)
                    .expect("we just got this path from the packages list");
                if prev_info.checksum != info.checksum {
                    return Err(Error::MismatchedShas {
                        ident: info.ident.clone(),
                        sha1: prev_info.checksum.clone().unwrap_or_default(),
                        sha2: info.checksum.clone().unwrap_or_default(),
                    }
                    .into());
                }
            }
        }
        Ok(Self { data, key_to_entry })
    }
}
impl PackageEntry {
    // Extracts version from key
    fn version(&self) -> &str {
        self.ident
            .rsplit_once('@')
            .map(|(_, version)| version)
            .unwrap_or(&self.ident)
    }
}

impl PackageInfo {
    pub fn all_dependencies(&self) -> impl Iterator<Item = (&str, &str)> {
        [
            self.dependencies.iter(),
            self.dev_dependencies.iter(),
            self.optional_dependencies.iter(),
            self.peer_dependencies.iter(),
        ]
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    const BASIC_LOCKFILE: &str = include_str!("../../fixtures/basic-bun.lock");
    const PATCH_LOCKFILE: &str = include_str!("../../fixtures/bun-patch.lock");

    #[test_case("", "turbo", "^2.3.3", "turbo@2.3.3" ; "root")]
    #[test_case("apps/docs", "is-odd", "3.0.1", "is-odd@3.0.1" ; "docs is odd")]
    #[test_case("apps/web", "is-odd", "3.0.0", "is-odd@3.0.0" ; "web is odd")]
    #[test_case("packages/ui", "node-plop/inquirer/rxjs/tslib", "^1.14.0", "tslib@1.14.1" ; "full key")]
    fn test_resolve_package(workspace: &str, name: &str, version: &str, expected: &str) {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();
        let result = lockfile
            .resolve_package(workspace, name, version)
            .unwrap()
            .unwrap();
        assert_eq!(result.key, expected);
    }

    const TURBO_GEN_DEPS: &[&str] = [
        "@turbo/gen/chalk",
        "@turbo/gen/minimatch",
        "@turbo/workspaces",
        "commander",
        "fs-extra",
        "inquirer",
        "node-plop",
        "proxy-agent",
        "ts-node",
        "update-check",
        "validate-npm-package-name",
    ]
    .as_slice();
    // Both @turbo/gen and log-symbols depend on the same version of chalk
    // log-symbols version wins out, but this is okay since they are the same exact
    // version of chalk.
    const TURBO_GEN_CHALK_DEPS: &[&str] = [
        "log-symbols/chalk/ansi-styles",
        "log-symbols/chalk/escape-string-regexp",
        "log-symbols/chalk/supports-color",
    ]
    .as_slice();
    const CHALK_DEPS: &[&str] = ["ansi-styles", "supports-color"].as_slice();

    #[test_case("@turbo/gen@1.13.4", TURBO_GEN_DEPS)]
    #[test_case("chalk@2.4.2", TURBO_GEN_CHALK_DEPS)]
    #[test_case("chalk@4.1.2", CHALK_DEPS)]
    fn test_all_dependencies(key: &str, expected: &[&str]) {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();
        let mut expected = expected.to_vec();
        expected.sort();

        let mut actual = lockfile
            .all_dependencies(key)
            .unwrap()
            .unwrap()
            .into_keys()
            .collect::<Vec<_>>();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_patch_is_captured_in_package() {
        let lockfile = BunLockfile::from_str(PATCH_LOCKFILE).unwrap();
        let pkg = lockfile
            .resolve_package("apps/b", "is-odd", "3.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            pkg,
            crate::Package::new("is-odd@3.0.0", "3.0.0+patches/is-odd@3.0.0.patch")
        );
    }

    #[test]
    fn test_failure_if_mismatched_keys() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "foo": "^1.0.0",
                        "bar": "^1.0.0",
                    }
                }
            },
            "packages": {
                "bar": ["bar@1.0.0", { "dependencies": { "shared": "^1.0.0" } }, "sha512-goodbye"],
                "bar/shared": ["shared@1.0.0", {}, "sha512-bar"],
                "foo": ["foo@1.0.0", { "dependencies": { "shared": "^1.0.0" } }, "sha512-hello"],
                "foo/shared": ["shared@1.0.0", { }, "sha512-foo"],
            }
        }))
        .unwrap();
        let lockfile = BunLockfile::from_str(&contents);
        assert!(lockfile.is_err(), "matching packages have differing shas");
    }
}
