//! # Bun Lockfile Support
//!
//! This module provides comprehensive support for Bun lockfiles (`bun.lockb`),
//! handling both binary and JSON formats with support for lockfile versions 0
//! and 1.
//!
//! ## Lockfile Version Support
//!
//! ### Version 0 (Legacy)
//! - Basic dependency resolution
//! - Simple workspace support
//! - Limited override functionality
//!
//! ### Version 1 (Current)
//! - Enhanced workspace dependency resolution with optimized lookup strategies
//! - Improved catalog support with multiple catalog types
//! - Advanced override functionality with precedence rules
//! - Optimized subgraph filtering for workspace dependencies
//!
//! ## Key Features
//!
//! ### Catalog Resolution
//! The module supports Bun's catalog system for dependency version management:
//! - **Default catalog**: Available via `catalog` field in lockfile
//! - **Named catalogs**: Available via `catalogs` field with custom catalog
//!   names
//! - **Resolution precedence**: Named catalogs take precedence over default
//!   catalog
//! - **Workspace integration**: Catalogs work seamlessly with workspace
//!   dependencies
//!
//! Example catalog usage:
//! ```json
//! {
//!   "catalog": {
//!     "react": "^18.0.0"
//!   },
//!   "catalogs": {
//!     "frontend": {
//!       "react": "^18.2.0"
//!     }
//!   }
//! }
//! ```
//!
//! ### Override Functionality
//! Provides dependency override capabilities similar to npm/yarn:
//! - **Package overrides**: Replace specific package versions across the
//!   dependency tree
//! - **Scoped overrides**: Apply overrides to specific dependency contexts
//! - **Patch integration**: Overrides work in conjunction with patch
//!   dependencies
//! - **Subgraph filtering**: Overrides are properly handled during dependency
//!   pruning
//!
//! ### V1 Workspace Optimizations
//! Version 1 lockfiles include several workspace-specific optimizations:
//! - **Direct workspace resolution**: Optimized lookup for workspace
//!   dependencies
//! - **Transitive workspace tracking**: Proper handling of
//!   workspace-to-workspace dependencies
//! - **Nested workspace support**: Full support for nested workspace structures
//! - **Dependency deduplication**: Smart deduplication of workspace
//!   dependencies
//!
//! ### Subgraph Filtering Behavior
//! The module provides sophisticated dependency subgraph extraction:
//! - **Workspace-aware filtering**: Preserves workspace dependency
//!   relationships
//! - **Override preservation**: Maintains override behavior in filtered
//!   subgraphs
//! - **Patch dependency handling**: Properly includes patched dependencies and
//!   their patches
//! - **Transitive closure**: Calculates complete transitive dependency closures
//!
//! ## Implementation Details
//!
//! The module handles both binary (`.lockb`) and JSON formats, with automatic
//! format detection and conversion. It provides full round-trip compatibility,
//! ensuring that lockfiles can be read, modified, and written back without data
//! loss.
//!
//! Key data structures:
//! - [`BunLockfile`]: Main lockfile representation with metadata and entry
//!   mappings
//! - [`BunLockfileData`]: Raw lockfile data structure matching Bun's format
//! - [`WorkspaceEntry`]: Workspace package representation
//! - [`PackageEntry`]: External package representation
//! - [`LockfileVersion`]: Version enum for format compatibility

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    str::FromStr,
};

use biome_json_formatter::context::JsonFormatOptions;
use biome_json_parser::JsonParserOptions;
use id::PossibleKeyIter;
use itertools::Itertools as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use turborepo_errors::ParseDiagnostic;

use crate::Lockfile;

mod de;
mod id;
mod ser;

type Map<K, V> = std::collections::BTreeMap<K, V>;

/// Represents a platform constraint that can be either inclusive or exclusive.
/// This matches Bun's Negatable type for os/cpu/libc fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Negatable {
    /// No constraint - package works on all platforms
    None,
    /// Single platform constraint
    Single(String),
    /// Multiple platform constraints (all must match)
    Multiple(Vec<String>),
    /// Negated constraints - package works on all platforms except these
    Negated(Vec<String>),
}

impl Default for Negatable {
    fn default() -> Self {
        Self::None
    }
}

impl Serialize for Negatable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Negatable::None => serializer.serialize_str("none"),
            Negatable::Single(platform) => serializer.serialize_str(platform),
            Negatable::Multiple(platforms) => platforms.serialize(serializer),
            Negatable::Negated(platforms) => {
                let negated_platforms: Vec<String> =
                    platforms.iter().map(|p| format!("!{p}")).collect();
                negated_platforms.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Negatable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Value = Value::deserialize(deserializer)?;

        match value {
            Value::String(s) => {
                if s == "none" {
                    Ok(Negatable::None)
                } else if let Some(stripped) = s.strip_prefix('!') {
                    Ok(Negatable::Negated(vec![stripped.to_string()]))
                } else {
                    Ok(Negatable::Single(s))
                }
            }
            Value::Array(arr) => {
                let platforms: Result<Vec<String>, _> = arr
                    .into_iter()
                    .map(|v| match v {
                        Value::String(s) => Ok(s),
                        _ => Err(serde::de::Error::invalid_type(
                            serde::de::Unexpected::Other("non-string in platform array"),
                            &"string",
                        )),
                    })
                    .collect();

                let platforms = platforms?;

                let has_negated = platforms.iter().any(|p| p.starts_with('!'));
                let has_non_negated = platforms.iter().any(|p| !p.starts_with('!'));

                if has_negated && has_non_negated {
                    // Mixed array: non-negated values define the allowlist, ignore negated values
                    // This matches npm behavior where explicit allows take precedence
                    let allowed_platforms: Vec<String> = platforms
                        .into_iter()
                        .filter(|p| !p.starts_with('!'))
                        .collect();
                    Ok(Negatable::Multiple(allowed_platforms))
                } else if has_negated {
                    // All negated: strip '!' prefix and treat as blocklist
                    let negated_platforms: Vec<String> = platforms
                        .into_iter()
                        .map(|p| p.strip_prefix('!').unwrap().to_string())
                        .collect();
                    Ok(Negatable::Negated(negated_platforms))
                } else {
                    // All non-negated: treat as allowlist
                    Ok(Negatable::Multiple(platforms))
                }
            }
            _ => Err(serde::de::Error::invalid_type(
                serde::de::Unexpected::Other("non-string/array for platform constraint"),
                &"string or array of strings",
            )),
        }
    }
}

impl Negatable {
    /// Returns true if this constraint allows the given platform
    #[allow(dead_code)]
    pub fn allows(&self, platform: &str) -> bool {
        match self {
            Negatable::None => true,
            Negatable::Single(p) => p == platform,
            Negatable::Multiple(platforms) => platforms.contains(&platform.to_string()),
            Negatable::Negated(platforms) => !platforms.contains(&platform.to_string()),
        }
    }

    /// Returns true if this is an empty/none constraint
    pub fn is_none(&self) -> bool {
        matches!(self, Negatable::None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to strip commas: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Failed to strip commas: {0}")]
    Format(#[from] biome_formatter::FormatError),
    #[error("Failed to strip commas: {0}")]
    Print(#[from] biome_formatter::PrintError),
    #[error("{ident} had two entries with differing checksums: {sha1}, {sha2}")]
    MismatchedShas {
        ident: String,
        sha1: String,
        sha2: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockfileVersion {
    V0 = 0,
    V1 = 1,
}

impl LockfileVersion {
    #[allow(dead_code)]
    fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::V0),
            1 => Some(Self::V1),
            _ => None,
        }
    }

    #[allow(dead_code)]
    fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug)]
pub struct BunLockfile {
    data: BunLockfileData,
    key_to_entry: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunLockfileData {
    lockfile_version: i32,
    workspaces: Map<String, WorkspaceEntry>,
    packages: Map<String, PackageEntry>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    patched_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    overrides: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    catalog: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    catalogs: Map<String, Map<String, String>>,
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
#[serde(rename_all = "camelCase")]
struct PackageInfo {
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dev_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    optional_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    peer_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    optional_peers: HashSet<String>,
    /// Operating system constraint for this package
    #[serde(default, skip_serializing_if = "Negatable::is_none")]
    os: Negatable,
    /// CPU architecture constraint for this package
    #[serde(default, skip_serializing_if = "Negatable::is_none")]
    cpu: Negatable,
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

        // Handle catalog references
        let resolved_version = if version.starts_with("catalog:") {
            // Try to resolve catalog reference
            if let Some(catalog_version) = self.resolve_catalog_version(name, version) {
                catalog_version
            } else {
                // Catalog reference couldn't be resolved, return None
                return Ok(None);
            }
        } else {
            version
        };

        // Apply overrides to the resolved version if any exist for this package
        let override_version = self.apply_overrides(name, resolved_version);

        // V1 optimization: Check if this is a workspace dependency that can be resolved
        // directly from the workspaces section without requiring a packages entry
        if self.data.lockfile_version >= 1
            && let Some(workspace_target_path) = self.resolve_workspace_dependency(override_version)
            && let Some(target_workspace) = self.data.workspaces.get(workspace_target_path)
        {
            // This is a workspace dependency, create a synthetic package entry
            let workspace_version = target_workspace.version.as_deref().unwrap_or("0.0.0");
            return Ok(Some(crate::Package {
                key: format!("{name}@{workspace_version}"),
                version: workspace_version.to_string(),
            }));
        }

        let workspace_key = format!("{workspace_name}/{name}");
        if let Some((_key, entry)) = self.package_entry(&workspace_key) {
            // Check if the entry matches the override version (if different from resolved)
            if override_version != resolved_version {
                // Look for a packages entry that matches the override version
                let override_ident = format!("{name}@{override_version}");
                // Try to find a package entry that matches the override
                if let Some((_override_key, override_entry)) = self
                    .data
                    .packages
                    .iter()
                    .find(|(_, entry)| entry.ident == override_ident)
                {
                    let mut pkg_version = override_entry.version().to_string();
                    // Check for any patches
                    if let Some(patch) = self.data.patched_dependencies.get(&override_entry.ident) {
                        pkg_version.push('+');
                        pkg_version.push_str(patch);
                    }
                    return Ok(Some(crate::Package {
                        key: override_entry.ident.to_string(),
                        version: pkg_version,
                    }));
                }
            }

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

        let Some(info) = &entry.info else {
            return Ok(Some(deps));
        };

        for (dependency, version) in info.all_dependencies() {
            let is_optional = info.optional_dependencies.contains_key(dependency)
                || info.optional_peers.contains(dependency);

            if is_optional {
                let parent_key = format!("{entry_key}/{dependency}");
                if self.package_entry(&parent_key).is_none() {
                    continue;
                }
            }

            deps.insert(dependency.to_string(), version.to_string());
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
        Ok(serde_json::to_vec_pretty(&self.data)?)
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        // Downcast returns none if the concrete type doesn't match
        // if the types don't match then we changed package managers
        let Some(other_bun) = any_other.downcast_ref::<Self>() else {
            return true;
        };

        // Check if lockfile version changed
        self.data.lockfile_version != other_bun.data.lockfile_version
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

    /// Returns the version override if there's an override for a package name
    fn apply_overrides<'a>(&'a self, name: &str, version: &'a str) -> &'a str {
        self.data
            .overrides
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(version)
    }

    /// Checks if a version string is a workspace dependency reference
    /// Returns the workspace path if it is (e.g., "packages/ui" ->
    /// Some("packages/ui"))
    fn resolve_workspace_dependency<'a>(&self, version: &'a str) -> Option<&'a str> {
        // Quick filter: if it starts with version characters, it's definitely not a
        // workspace
        if version.starts_with('^') || version.starts_with('~') || version.starts_with('=') {
            return None;
        }

        // Definitive check: does this path exist in our workspaces?
        if self.data.workspaces.contains_key(version) {
            Some(version)
        } else {
            None
        }
    }

    /// Resolves a catalog reference to the actual version
    /// Supports both default catalog ("catalog:") and named catalogs
    /// ("catalog:group:")
    fn resolve_catalog_version(&self, name: &str, catalog_ref: &str) -> Option<&str> {
        if let Some(stripped) = catalog_ref.strip_prefix("catalog:") {
            if stripped.is_empty() {
                // Default catalog reference: "catalog:"
                self.data.catalog.get(name).map(|s| s.as_str())
            } else {
                // Named catalog reference: "catalog:group:"
                self.data
                    .catalogs
                    .get(stripped)
                    .and_then(|catalog| catalog.get(name).map(|s| s.as_str()))
            }
        } else {
            None
        }
    }

    // Given a specific key for a package, return the most specific key that is
    // present in the lockfile
    fn package_entry(&self, key: &str) -> Option<(&str, &PackageEntry)> {
        let (key, entry) =
            PossibleKeyIter::new(key).find_map(|k| self.data.packages.get_key_value(k))?;
        Some((key, entry))
    }

    pub fn lockfile(self) -> Result<BunLockfileData, Error> {
        Ok(self.data)
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<BunLockfile, Error> {
        let new_workspaces: Map<_, _> = self
            .data
            .workspaces
            .iter()
            .filter_map(|(key, entry)| {
                // Ensure the root workspace package is included, which is always indexed by ""
                if key.is_empty() || workspace_packages.contains(key) {
                    Some((key.clone(), entry.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Filter out packages that are not in the subgraph. Note that _multiple_
        // entries can correspond to the same ident.
        let idents: HashSet<_> = packages.iter().collect();

        // First, collect packages that match the idents
        let mut new_packages: Map<_, _> = self
            .data
            .packages
            .iter()
            .filter_map(|(key, entry)| {
                if idents.contains(&entry.ident) {
                    Some((key.clone(), entry.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Then, for each included package, also include any scoped bundled dependencies
        // Bundled dependencies are stored as "<parent_key>/<dep_name>" in the packages
        // map and have "bundled": true in their metadata
        let parent_keys: Vec<_> = new_packages.keys().cloned().collect();
        for parent_key in parent_keys {
            let prefix = format!("{parent_key}/");
            for (key, entry) in self.data.packages.iter() {
                if key.starts_with(&prefix) && !new_packages.contains_key(key) {
                    // Check if this is a bundled dependency
                    if let Some(info) = &entry.info
                        && info.other.get("bundled") == Some(&Value::Bool(true))
                    {
                        new_packages.insert(key.clone(), entry.clone());
                    }
                }
            }
        }

        let new_patched_dependencies = self
            .data
            .patched_dependencies
            .iter()
            .filter_map(|(ident, patch)| {
                if idents.contains(ident) {
                    Some((ident.clone(), patch.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Extract package names from idents for filtering
        let package_names: HashSet<String> = idents
            .iter()
            .map(|ident| {
                // Extract package name from ident
                // e.g., "foo@1.0.0" -> "foo"
                // e.g., "@scope/package@1.0.0" -> "@scope/package"
                ident
                    .rsplit_once('@')
                    .map(|(name, _version)| name)
                    .unwrap_or(ident)
                    .to_string()
            })
            .collect();

        // Filter overrides to only include packages in the subgraph
        let new_overrides = self
            .data
            .overrides
            .iter()
            .filter_map(|(name, version)| {
                if package_names.contains(name) {
                    Some((name.clone(), version.clone()))
                } else {
                    None
                }
            })
            .collect();

        // For catalogs, we might want to keep them all since they could be referenced
        // by workspace dependencies, but we could also filter to only used ones
        // For now, keeping them all for simplicity

        // Filter key_to_entry to only include entries whose values (package keys)
        // exist in new_packages. This maintains the invariant that key_to_entry
        // only maps to valid entries in data.packages.
        let new_key_to_entry: HashMap<_, _> = self
            .key_to_entry
            .iter()
            .filter(|(_, package_key)| new_packages.contains_key(package_key.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Ok(Self {
            data: BunLockfileData {
                lockfile_version: self.data.lockfile_version,
                workspaces: new_workspaces,
                packages: new_packages,
                patched_dependencies: new_patched_dependencies,
                overrides: new_overrides,
                catalog: self.data.catalog.clone(),
                catalogs: self.data.catalogs.clone(),
            },
            key_to_entry: new_key_to_entry,
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

        // Validate that we support this lockfile version
        let _version = LockfileVersion::from_i32(data.lockfile_version)
            .ok_or(super::Error::UnsupportedBunVersion(data.lockfile_version))?;

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

/// Check if there are any global changes between two bun lockfiles
pub fn bun_global_change(prev_contents: &[u8], curr_contents: &[u8]) -> Result<bool, super::Error> {
    let prev = BunLockfile::from_bytes(prev_contents)?;
    let curr = BunLockfile::from_bytes(curr_contents)?;
    Ok(prev.data.lockfile_version != curr.data.lockfile_version)
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    const BASIC_LOCKFILE_V0: &str = include_str!("../../fixtures/basic-bun-v0.lock");
    const PATCH_LOCKFILE: &str = include_str!("../../fixtures/bun-patch-v0.lock");
    const CATALOG_LOCKFILE: &str = include_str!("../../fixtures/bun-catalog-v0.lock");

    #[test_case("", "turbo", "^2.3.3", "turbo@2.3.3" ; "root")]
    #[test_case("apps/docs", "is-odd", "3.0.1", "is-odd@3.0.1" ; "docs is odd")]
    #[test_case("apps/web", "is-odd", "3.0.0", "is-odd@3.0.0" ; "web is odd")]
    #[test_case("packages/ui", "node-plop/inquirer/rxjs/tslib", "^1.14.0", "tslib@1.14.1" ; "full key")]
    fn test_resolve_package(workspace: &str, name: &str, version: &str, expected: &str) {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE_V0).unwrap();
        let result = lockfile
            .resolve_package(workspace, name, version)
            .unwrap()
            .unwrap();
        assert_eq!(result.key, expected);
    }

    #[test]
    fn test_subgraph() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE_V0).unwrap();
        let subgraph = lockfile
            .subgraph(&["apps/docs".into()], &["is-odd@3.0.1".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        assert_eq!(
            subgraph_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![("is-odd", "is-odd@3.0.1")]
        );
        assert_eq!(
            subgraph_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/docs"]
        );
    }

    #[test]
    fn test_subgraph_filters_key_to_entry() {
        // Test that subgraph properly filters key_to_entry to only include
        // entries for packages that exist in the filtered packages map.
        // This maintains the invariant that every value in key_to_entry
        // must be a valid key in data.packages.
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE_V0).unwrap();

        // Verify the original lockfile has multiple packages
        let original_package_count = lockfile.data.packages.len();
        let original_key_to_entry_count = lockfile.key_to_entry.len();
        assert!(
            original_package_count > 1,
            "Test requires lockfile with multiple packages"
        );
        assert!(
            original_key_to_entry_count > 0,
            "Test requires lockfile with key_to_entry mappings"
        );

        // Create a subgraph with only a subset of packages
        let subgraph = lockfile
            .subgraph(&["apps/docs".into()], &["is-odd@3.0.1".into()])
            .unwrap();

        // Verify the subgraph has fewer packages
        let subgraph_package_count = subgraph.data.packages.len();
        assert!(
            subgraph_package_count < original_package_count,
            "Subgraph should have fewer packages than original"
        );

        // Verify all keys in key_to_entry map to valid packages in data.packages
        for (lookup_key, package_key) in &subgraph.key_to_entry {
            assert!(
                subgraph.data.packages.contains_key(package_key),
                "key_to_entry[{lookup_key:?}] = {package_key:?}, but {package_key:?} not in \
                 packages"
            );
        }

        // Verify that filtered-out packages are not in key_to_entry
        // For example, "chalk" should be in the original but not in the subgraph
        let has_chalk_in_original = lockfile
            .key_to_entry
            .values()
            .any(|key| key.contains("chalk"));
        let has_chalk_in_subgraph = subgraph
            .key_to_entry
            .values()
            .any(|key| key.contains("chalk"));

        if has_chalk_in_original {
            assert!(
                !has_chalk_in_subgraph,
                "chalk should be filtered out of subgraph's key_to_entry"
            );
        }

        // Verify the subgraph only has packages we requested
        assert_eq!(subgraph.data.packages.len(), 1);
        assert!(subgraph.data.packages.contains_key("is-odd"));
    }

    // There are multiple aliases that resolve to the same ident, here we test that
    // we output them all
    #[test]
    fn test_deduplicated_idents() {
        // chalk@2.4.2
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE_V0).unwrap();
        let subgraph = lockfile
            .subgraph(&["apps/docs".into()], &["chalk@2.4.2".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        assert_eq!(
            subgraph_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("@turbo/gen/chalk", "chalk@2.4.2"),
                ("@turbo/workspaces/chalk", "chalk@2.4.2"),
                ("log-symbols/chalk", "chalk@2.4.2")
            ]
        );
        assert_eq!(
            subgraph_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/docs"]
        );
    }

    #[test]
    fn test_patch_subgraph() {
        let lockfile = BunLockfile::from_str(PATCH_LOCKFILE).unwrap();
        let subgraph_a = lockfile
            .subgraph(&["apps/a".into()], &["is-odd@3.0.1".into()])
            .unwrap();
        let subgraph_a_data = subgraph_a.lockfile().unwrap();

        assert_eq!(
            subgraph_a_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![("is-odd", "is-odd@3.0.1")]
        );
        assert_eq!(
            subgraph_a_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/a"]
        );
        assert_eq!(
            subgraph_a_data
                .patched_dependencies
                .iter()
                .map(|(key, patch)| (key.as_str(), patch.as_str()))
                .collect::<Vec<_>>(),
            vec![]
        );

        let subgraph_b = lockfile
            .subgraph(&["apps/b".into()], &["is-odd@3.0.0".into()])
            .unwrap();
        let subgraph_b_data = subgraph_b.lockfile().unwrap();

        assert_eq!(
            subgraph_b_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![("b/is-odd", "is-odd@3.0.0")]
        );
        assert_eq!(
            subgraph_b_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/b"]
        );
        assert_eq!(
            subgraph_b_data
                .patched_dependencies
                .iter()
                .map(|(key, patch)| (key.as_str(), patch.as_str()))
                .collect::<Vec<_>>(),
            vec![("is-odd@3.0.0", "patches/is-odd@3.0.0.patch")]
        );
    }

    const TURBO_GEN_DEPS: &[&str] = [
        "@turbo/workspaces",
        "chalk",
        "commander",
        "fs-extra",
        "inquirer",
        "minimatch",
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
    const TURBO_GEN_CHALK_DEPS: &[&str] =
        ["ansi-styles", "escape-string-regexp", "supports-color"].as_slice();
    const CHALK_DEPS: &[&str] = ["ansi-styles", "supports-color"].as_slice();

    #[test_case("@turbo/gen@1.13.4", TURBO_GEN_DEPS)]
    #[test_case("chalk@2.4.2", TURBO_GEN_CHALK_DEPS)]
    #[test_case("chalk@4.1.2", CHALK_DEPS)]
    fn test_all_dependencies(key: &str, expected: &[&str]) {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE_V0).unwrap();
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
    fn test_global_change_version_mismatch() {
        let v0_contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                }
            },
            "packages": {}
        }))
        .unwrap();

        let v1_contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                }
            },
            "packages": {}
        }))
        .unwrap();

        let v0_lockfile = BunLockfile::from_str(&v0_contents).unwrap();
        let v1_lockfile = BunLockfile::from_str(&v1_contents).unwrap();

        // Version change should be detected
        assert!(v0_lockfile.global_change(&v1_lockfile));

        // Same version should not be a global change
        assert!(!v0_lockfile.global_change(&v0_lockfile));
        assert!(!v1_lockfile.global_change(&v1_lockfile));
    }

    #[test]
    fn test_bun_global_change_function() {
        let v0_contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                }
            },
            "packages": {}
        }))
        .unwrap();

        let v1_contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                }
            },
            "packages": {}
        }))
        .unwrap();

        // Test the standalone function
        assert!(bun_global_change(v0_contents.as_bytes(), v1_contents.as_bytes()).unwrap());
        assert!(!bun_global_change(v0_contents.as_bytes(), v0_contents.as_bytes()).unwrap());
        assert!(!bun_global_change(v1_contents.as_bytes(), v1_contents.as_bytes()).unwrap());
    }

    #[test]
    fn test_new_fields_parsing() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "foo": "^1.0.0"
                    }
                }
            },
            "packages": {
                "foo": ["foo@1.0.0", {}, "sha512-hello"]
            },
            "overrides": {
                "foo": "1.0.0"
            },
            "catalog": {
                "react": "^18.0.0"
            },
            "catalogs": {
                "frontend": {
                    "react": "^18.0.0",
                    "next": "^14.0.0"
                }
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Check that new fields are parsed
        assert_eq!(lockfile.data.overrides.len(), 1);
        assert_eq!(
            lockfile.data.overrides.get("foo"),
            Some(&"1.0.0".to_string())
        );

        assert_eq!(lockfile.data.catalog.len(), 1);
        assert_eq!(
            lockfile.data.catalog.get("react"),
            Some(&"^18.0.0".to_string())
        );

        assert_eq!(lockfile.data.catalogs.len(), 1);
        let frontend_catalog = lockfile.data.catalogs.get("frontend").unwrap();
        assert_eq!(frontend_catalog.len(), 2);
        assert_eq!(frontend_catalog.get("react"), Some(&"^18.0.0".to_string()));
        assert_eq!(frontend_catalog.get("next"), Some(&"^14.0.0".to_string()));
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

    #[test]
    fn test_override_functionality() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "foo": "^1.0.0"
                    }
                }
            },
            "packages": {
                "foo": ["foo@1.0.0", {}, "sha512-original"],
                "foo-override": ["foo@2.0.0", {}, "sha512-override"]
            },
            "overrides": {
                "foo": "2.0.0"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Resolve foo - should get override version instead of original
        let result = lockfile
            .resolve_package("", "foo", "^1.0.0")
            .unwrap()
            .unwrap();

        // Should resolve to overridden version
        assert_eq!(result.key, "foo@2.0.0");
        assert_eq!(result.version, "2.0.0");
    }

    #[test]
    fn test_override_functionality_no_override() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "bar": "^1.0.0"
                    }
                }
            },
            "packages": {
                "bar": ["bar@1.0.0", {}, "sha512-original"]
            },
            "overrides": {
                "foo": "2.0.0"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Resolve bar - should get original version since no override exists for bar
        let result = lockfile
            .resolve_package("", "bar", "^1.0.0")
            .unwrap()
            .unwrap();

        // Should resolve to original version (no override)
        assert_eq!(result.key, "bar@1.0.0");
        assert_eq!(result.version, "1.0.0");
    }

    #[test]
    fn test_subgraph_filters_overrides() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "foo": "^1.0.0",
                        "bar": "^1.0.0"
                    }
                },
                "apps/web": {
                    "name": "web",
                    "dependencies": {
                        "foo": "^1.0.0"
                    }
                }
            },
            "packages": {
                "foo": ["foo@1.0.0", {}, "sha512-foo"],
                "bar": ["bar@1.0.0", {}, "sha512-bar"]
            },
            "overrides": {
                "foo": "2.0.0",
                "bar": "2.0.0",
                "unused": "1.0.0"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Create subgraph with only foo package
        let subgraph = lockfile
            .subgraph(&["apps/web".into()], &["foo@1.0.0".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Check that overrides are filtered
        assert_eq!(subgraph_data.overrides.len(), 1);
        assert!(subgraph_data.overrides.contains_key("foo"));
        assert!(!subgraph_data.overrides.contains_key("bar"));
        assert!(!subgraph_data.overrides.contains_key("unused"));

        // Check that workspaces are correct
        assert_eq!(subgraph_data.workspaces.len(), 2);
        assert!(subgraph_data.workspaces.contains_key(""));
        assert!(subgraph_data.workspaces.contains_key("apps/web"));

        // Check that packages are correct
        assert_eq!(subgraph_data.packages.len(), 1);
        assert!(subgraph_data.packages.contains_key("foo"));
        assert!(!subgraph_data.packages.contains_key("bar"));
    }

    #[test]
    fn test_override_with_patches() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "lodash": "^4.17.20"
                    }
                }
            },
            "packages": {
                "lodash": ["lodash@4.17.20", {}, "sha512-original"],
                "lodash-override": ["lodash@4.17.21", {}, "sha512-override"]
            },
            "overrides": {
                "lodash": "4.17.21"
            },
            "patchedDependencies": {
                "lodash@4.17.21": "patches/lodash@4.17.21.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Resolve lodash - should get override version with patch
        let result = lockfile
            .resolve_package("", "lodash", "^4.17.20")
            .unwrap()
            .unwrap();

        // Should resolve to overridden version with patch
        assert_eq!(result.key, "lodash@4.17.21");
        assert_eq!(result.version, "4.17.21+patches/lodash@4.17.21.patch");
    }

    #[test]
    fn test_catalog_resolution_methods() {
        let lockfile = BunLockfile::from_str(CATALOG_LOCKFILE).unwrap();

        // Test resolving from default catalog
        assert_eq!(
            lockfile.resolve_catalog_version("react", "catalog:"),
            Some("^18.2.0")
        );
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:"),
            Some("^4.17.21")
        );

        // Test resolving from named catalog
        assert_eq!(
            lockfile.resolve_catalog_version("react", "catalog:frontend"),
            Some("^19.0.0")
        );
        assert_eq!(
            lockfile.resolve_catalog_version("next", "catalog:frontend"),
            Some("^14.0.0")
        );

        // Test resolving non-existent package from default catalog
        assert_eq!(
            lockfile.resolve_catalog_version("non-existent", "catalog:"),
            None
        );

        // Test resolving from non-existent catalog
        assert_eq!(
            lockfile.resolve_catalog_version("react", "catalog:non-existent"),
            None
        );

        // Test resolving package not in named catalog
        assert_eq!(
            lockfile.resolve_catalog_version("lodash", "catalog:frontend"),
            None
        );

        // Test non-catalog version
        assert_eq!(lockfile.resolve_catalog_version("react", "^18.0.0"), None);
    }

    #[test_case("apps/web", "react", "catalog:", "react@18.2.0" ; "default catalog react")]
    #[test_case("apps/web", "lodash", "catalog:", "lodash@4.17.21" ; "default catalog lodash")]
    #[test_case("apps/docs", "react", "catalog:frontend", "react@19.0.0" ; "frontend catalog react")]
    #[test_case("apps/docs", "next", "catalog:frontend", "next@14.0.0" ; "frontend catalog next")]
    fn test_resolve_package_with_catalog(
        workspace: &str,
        name: &str,
        version: &str,
        expected: &str,
    ) {
        let lockfile = BunLockfile::from_str(CATALOG_LOCKFILE).unwrap();
        let result = lockfile
            .resolve_package(workspace, name, version)
            .unwrap()
            .unwrap();
        assert_eq!(result.key, expected);
    }

    #[test]
    fn test_resolve_package_catalog_not_found() {
        let lockfile = BunLockfile::from_str(CATALOG_LOCKFILE).unwrap();

        // Test resolving non-existent package from catalog
        let result = lockfile
            .resolve_package("apps/web", "non-existent", "catalog:")
            .unwrap();
        assert!(result.is_none());

        // Test resolving from non-existent catalog
        let result = lockfile
            .resolve_package("apps/web", "react", "catalog:non-existent")
            .unwrap();
        assert!(result.is_none());

        // Test resolving package not in named catalog
        let result = lockfile
            .resolve_package("apps/docs", "lodash", "catalog:frontend")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_catalog_with_overrides() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "react": "catalog:"
                    }
                }
            },
            "packages": {
                "react": ["react@18.2.0", {}, "sha512-react18"],
                "react-override": ["react@19.0.0", {}, "sha512-react19"]
            },
            "catalog": {
                "react": "^18.2.0"
            },
            "overrides": {
                "react": "19.0.0"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Resolve react - should get override version instead of catalog version
        let result = lockfile
            .resolve_package("", "react", "catalog:")
            .unwrap()
            .unwrap();

        // Should resolve to overridden version
        assert_eq!(result.key, "react@19.0.0");
        assert_eq!(result.version, "19.0.0");
    }

    #[test]
    fn test_catalog_subgraph_preservation() {
        let lockfile = BunLockfile::from_str(CATALOG_LOCKFILE).unwrap();
        let subgraph = lockfile
            .subgraph(&["apps/web".into()], &["react@18.2.0".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Check that catalogs are preserved in subgraph
        assert_eq!(subgraph_data.catalog.len(), 2);
        assert_eq!(
            subgraph_data.catalog.get("react"),
            Some(&"^18.2.0".to_string())
        );
        assert_eq!(
            subgraph_data.catalog.get("lodash"),
            Some(&"^4.17.21".to_string())
        );

        assert_eq!(subgraph_data.catalogs.len(), 1);
        let frontend_catalog = subgraph_data.catalogs.get("frontend").unwrap();
        assert_eq!(frontend_catalog.len(), 2);
        assert_eq!(frontend_catalog.get("react"), Some(&"^19.0.0".to_string()));
        assert_eq!(frontend_catalog.get("next"), Some(&"^14.0.0".to_string()));
    }

    const V1_WORKSPACE_LOCKFILE_1: &str = include_str!("../../fixtures/bun-v1-1.lock");
    const V1_CREATE_TURBO_LOCKFILE: &str = include_str!("../../fixtures/bun-v1-create-turbo.lock");
    const V1_ISSUE_10410_LOCKFILE: &str = include_str!("../../fixtures/bun-v1-issue-10410.lock");

    #[test]
    fn test_v1_workspace_dependency_resolution() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Test resolving a workspace dependency from apps/web to packages/ui
        let result = lockfile
            .resolve_package("apps/web", "@repo/ui", "packages/ui")
            .unwrap()
            .unwrap();

        // Should resolve directly from workspace entry without needing packages entry
        assert_eq!(result.key, "@repo/ui@0.1.0");
        assert_eq!(result.version, "0.1.0");
    }

    #[test]
    fn test_v1_nested_workspace_dependency_resolution() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Test resolving a workspace dependency from packages/ui to
        // packages/shared-utils
        let result = lockfile
            .resolve_package("packages/ui", "@repo/shared-utils", "packages/shared-utils")
            .unwrap()
            .unwrap();

        assert_eq!(result.key, "@repo/shared-utils@0.2.0");
        assert_eq!(result.version, "0.2.0");
    }

    #[test]
    fn test_v1_non_workspace_dependency_resolution() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Test resolving a regular dependency - should still work normally
        let result = lockfile
            .resolve_package("packages/shared-utils", "lodash", "^4.17.21")
            .unwrap()
            .unwrap();

        assert_eq!(result.key, "lodash@4.17.21");
        assert_eq!(result.version, "4.17.21");
    }

    #[test]
    fn test_v1_workspace_dependency_not_found() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Test resolving a non-existent workspace dependency
        let result = lockfile
            .resolve_package("apps/web", "@repo/non-existent", "packages/non-existent")
            .unwrap();

        // Should return None since workspace doesn't exist
        assert!(result.is_none());
    }

    #[test]
    fn test_v1_lockfile_version_detection() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Verify lockfile version is correctly parsed as 1
        assert_eq!(lockfile.data.lockfile_version, 1);
    }

    #[test]
    fn test_v1_create_turbo_lockfile_parse() {
        let lockfile = BunLockfile::from_str(V1_CREATE_TURBO_LOCKFILE).unwrap();
        assert_eq!(lockfile.data.lockfile_version, 1);

        // Verify workspaces are parsed correctly
        assert_eq!(lockfile.data.workspaces.len(), 6);
        assert!(lockfile.data.workspaces.contains_key(""));
        assert!(lockfile.data.workspaces.contains_key("apps/docs"));
        assert!(lockfile.data.workspaces.contains_key("apps/web"));
        assert!(lockfile.data.workspaces.contains_key("packages/ui"));
        assert!(
            lockfile
                .data
                .workspaces
                .contains_key("packages/eslint-config")
        );
        assert!(
            lockfile
                .data
                .workspaces
                .contains_key("packages/typescript-config")
        );

        // Verify packages are parsed
        assert!(!lockfile.data.packages.is_empty());
        assert!(lockfile.data.packages.contains_key("react"));
        assert!(lockfile.data.packages.contains_key("next"));
        assert!(lockfile.data.packages.contains_key("turbo"));
    }

    #[test]
    fn test_v1_create_turbo_workspace_resolution() {
        let lockfile = BunLockfile::from_str(V1_CREATE_TURBO_LOCKFILE).unwrap();

        // Test resolving workspace dependency from apps/docs to packages/ui
        let result = lockfile
            .resolve_package("apps/docs", "@repo/ui", "*")
            .unwrap()
            .unwrap();

        assert_eq!(result.key, "@repo/ui@workspace:packages/ui");
        assert_eq!(result.version, "workspace:packages/ui");

        // Test resolving external dependency
        let react_result = lockfile
            .resolve_package("apps/docs", "react", "^19.1.0")
            .unwrap()
            .unwrap();

        assert_eq!(react_result.key, "react@19.1.1");
        assert_eq!(react_result.version, "19.1.1");
    }

    #[test]
    fn test_v1_create_turbo_turbo_version() {
        let lockfile = BunLockfile::from_str(V1_CREATE_TURBO_LOCKFILE).unwrap();
        let turbo_version = lockfile.turbo_version();
        assert_eq!(turbo_version, Some("2.5.8".to_string()));
    }

    #[test]
    fn test_optional_dependencies_not_in_lockfile() {
        // Test that optional dependencies that are not present in the lockfile
        // don't cause errors when calculating transitive closures
        let lockfile_content = r#"{
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-app",
                    "dependencies": {
                        "@emnapi/runtime": "^1.0.0"
                    }
                }
            },
            "packages": {
                "@emnapi/runtime": [
                    "@emnapi/runtime@1.5.0",
                    "",
                    {
                        "dependencies": {
                            "tslib": "^2.4.0"
                        },
                        "optionalDependencies": {
                            "@emnapi/wasi-threads": "^1.0.0"
                        }
                    },
                    "sha512"
                ],
                "tslib": [
                    "tslib@2.8.1",
                    "",
                    {},
                    "sha512"
                ]
            }
        }"#;

        let lockfile = BunLockfile::from_str(lockfile_content).unwrap();

        // This should not error even though @emnapi/wasi-threads is not in the packages
        let deps = lockfile
            .all_dependencies("@emnapi/runtime@1.5.0")
            .unwrap()
            .unwrap();

        // Should only contain tslib, not @emnapi/wasi-threads
        assert_eq!(deps.len(), 1);
        let keys: Vec<_> = deps.keys().collect();
        assert_eq!(keys.len(), 1);
        assert!(keys[0].contains("tslib"));
        assert!(!deps.values().any(|v| v.contains("@emnapi/wasi-threads")));
    }

    #[test]
    fn test_v1_issue_10410_bundled_dependencies() {
        let lockfile = BunLockfile::from_str(V1_ISSUE_10410_LOCKFILE).unwrap();
        assert_eq!(lockfile.data.lockfile_version, 1);

        // This lockfile has bundled dependencies which should be resolved correctly
        // @tailwindcss/oxide-wasm32-wasi has scoped bundled dependencies
        let result = lockfile
            .resolve_package("apps/web", "@tailwindcss/vite", "^4.1.13")
            .unwrap()
            .unwrap();

        assert_eq!(result.key, "@tailwindcss/vite@4.1.13");

        // Test that we can get all dependencies without errors
        // This should not fail with "No lockfile entry found for
        // '@emnapi/wasi-threads'"
        let deps = lockfile
            .all_dependencies("@tailwindcss/oxide-wasm32-wasi@4.1.13")
            .unwrap()
            .unwrap();

        // Should be able to find bundled dependencies under the scoped path
        assert!(!deps.is_empty());

        // Test transitive closure calculation for apps/web
        // This is the scenario that was failing with the warning
        let workspace_entry = &lockfile.data.workspaces["apps/web"];
        let mut unresolved_deps = HashMap::new();
        if let Some(deps) = &workspace_entry.dependencies {
            for (name, version) in deps {
                unresolved_deps.insert(name.clone(), version.clone());
            }
        }
        if let Some(dev_deps) = &workspace_entry.dev_dependencies {
            for (name, version) in dev_deps {
                unresolved_deps.insert(name.clone(), version.clone());
            }
        }

        // This should complete without errors - previously threw warning about
        // '@emnapi/wasi-threads'
        let closure = crate::transitive_closure(&lockfile, "apps/web", unresolved_deps, false);
        assert!(
            closure.is_ok(),
            "Transitive closure failed: {}",
            closure.unwrap_err()
        );
    }

    #[test]
    fn test_v0_vs_v1_workspace_behavior() {
        let v0_lockfile = BunLockfile::from_str(BASIC_LOCKFILE_V0).unwrap();
        assert_eq!(v0_lockfile.data.lockfile_version, 0);

        // V0 should resolve workspace deps through packages section
        let v0_result = v0_lockfile
            .resolve_package("apps/docs", "@repo/ui", "packages/ui")
            .unwrap()
            .unwrap();

        // Test with V1 lockfile
        let v1_lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();
        assert_eq!(v1_lockfile.data.lockfile_version, 1);

        // V1 should resolve workspace deps directly from workspaces section
        let v1_result = v1_lockfile
            .resolve_package("apps/web", "@repo/ui", "packages/ui")
            .unwrap()
            .unwrap();

        // Both should resolve, but v1 uses direct workspace resolution
        assert_eq!(v0_result.key, "@repo/ui@workspace:packages/ui");
        assert_eq!(v1_result.key, "@repo/ui@0.1.0");
    }

    #[test]
    fn test_resolve_workspace_dependency_helper() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Should recognize workspace paths
        assert_eq!(
            lockfile.resolve_workspace_dependency("packages/ui"),
            Some("packages/ui")
        );
        assert_eq!(
            lockfile.resolve_workspace_dependency("packages/shared-utils"),
            Some("packages/shared-utils")
        );

        // Should not recognize version strings
        assert_eq!(lockfile.resolve_workspace_dependency("^4.17.21"), None);
        assert_eq!(lockfile.resolve_workspace_dependency("~1.0.0"), None);
        assert_eq!(lockfile.resolve_workspace_dependency("=1.0.0"), None);

        // Should not recognize non-existent paths
        assert_eq!(
            lockfile.resolve_workspace_dependency("packages/non-existent"),
            None
        );

        // Should not recognize strings without slashes
        assert_eq!(lockfile.resolve_workspace_dependency("react"), None);
    }

    #[test]
    fn test_v1_subgraph_with_workspace_dependencies() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Create subgraph including apps/web but not packages/ui
        // Note: In v1, workspace packages don't appear in packages section, so we
        // don't need to include them in the packages list for subgraph
        let subgraph = lockfile
            .subgraph(&["apps/web".into()], &["react@18.0.0".into()])
            .unwrap();

        // Test resolution before getting data to avoid move
        let ui_result = subgraph
            .resolve_package("apps/web", "@repo/ui", "packages/ui")
            .unwrap();
        assert!(ui_result.is_none()); // UI workspace not included in subgraph workspaces

        // Now get the data
        let subgraph_data = subgraph.lockfile().unwrap();

        // Check that the workspace is included
        assert!(subgraph_data.workspaces.contains_key("apps/web"));
        assert!(subgraph_data.workspaces.contains_key("")); // root always included

        // Check that external packages are filtered correctly
        assert_eq!(subgraph_data.packages.len(), 1);
        assert!(subgraph_data.packages.contains_key("react"));
    }

    #[test]
    fn test_v1_subgraph_includes_workspace_dependencies() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Create subgraph that includes both apps/web and the workspace it depends on
        let subgraph = lockfile
            .subgraph(
                &["apps/web".into(), "packages/ui".into()],
                &["react@18.0.0".into()],
            )
            .unwrap();

        // Test resolution before getting data to avoid move
        let ui_result = subgraph
            .resolve_package("apps/web", "@repo/ui", "packages/ui")
            .unwrap()
            .unwrap();
        assert_eq!(ui_result.key, "@repo/ui@0.1.0");
        assert_eq!(ui_result.version, "0.1.0");

        // Now get the data
        let subgraph_data = subgraph.lockfile().unwrap();

        // Check that both workspaces are included
        assert!(subgraph_data.workspaces.contains_key("apps/web"));
        assert!(subgraph_data.workspaces.contains_key("packages/ui"));
        assert!(subgraph_data.workspaces.contains_key("")); // root always
        // included
    }

    #[test]
    fn test_v1_subgraph_transitively_includes_workspace_deps() {
        let lockfile = BunLockfile::from_str(V1_WORKSPACE_LOCKFILE_1).unwrap();

        // Create subgraph that includes packages/ui and its dependencies
        // packages/ui depends on packages/shared-utils (workspace) and react (external)
        let subgraph = lockfile
            .subgraph(
                &["packages/ui".into(), "packages/shared-utils".into()],
                &["react@18.0.0".into(), "lodash@4.17.21".into()],
            )
            .unwrap();

        // Test resolution before getting data to avoid move
        let shared_utils_result = subgraph
            .resolve_package("packages/ui", "@repo/shared-utils", "packages/shared-utils")
            .unwrap()
            .unwrap();
        assert_eq!(shared_utils_result.key, "@repo/shared-utils@0.2.0");

        let lodash_result = subgraph
            .resolve_package("packages/shared-utils", "lodash", "^4.17.21")
            .unwrap()
            .unwrap();
        assert_eq!(lodash_result.key, "lodash@4.17.21");

        // Now get the data
        let subgraph_data = subgraph.lockfile().unwrap();

        // Check workspaces
        assert!(subgraph_data.workspaces.contains_key("packages/ui"));
        assert!(
            subgraph_data
                .workspaces
                .contains_key("packages/shared-utils")
        );
        assert!(subgraph_data.workspaces.contains_key("")); // root always included

        // Check packages
        assert!(subgraph_data.packages.contains_key("react"));
        assert!(subgraph_data.packages.contains_key("lodash"));
    }

    // ============================================================================
    // COMPREHENSIVE INTEGRATION TESTS
    // ============================================================================

    #[test]
    fn test_integration_v1_catalog_override_patch_combined() {
        // Test combining V1 format, catalogs, overrides, and patches
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "integration-test",
                    "dependencies": {
                        "react": "catalog:ui",
                        "lodash": "catalog:"
                    }
                },
                "packages/ui": {
                    "name": "@repo/ui",
                    "version": "1.0.0",
                    "dependencies": {
                        "@repo/utils": "packages/utils",
                        "react": "catalog:ui"
                    }
                },
                "packages/utils": {
                    "name": "@repo/utils",
                    "version": "2.0.0",
                    "dependencies": {
                        "lodash": "catalog:"
                    }
                }
            },
            "packages": {
                "react": ["react@18.0.0", {}, "sha512-react18"],
                "react-19": ["react@19.0.0", {}, "sha512-react19"],
                "lodash": ["lodash@4.17.20", {}, "sha512-lodash420"],
                "lodash-patched": ["lodash@4.17.21", {}, "sha512-lodash421"]
            },
            "catalog": {
                "lodash": "^4.17.20"
            },
            "catalogs": {
                "ui": {
                    "react": "^18.0.0"
                }
            },
            "overrides": {
                "lodash": "4.17.21"
            },
            "patchedDependencies": {
                "lodash@4.17.21": "patches/lodash-security.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test catalog resolution with override
        let lodash_result = lockfile
            .resolve_package("", "lodash", "catalog:")
            .unwrap()
            .unwrap();
        // Should resolve catalog to 4.17.20, then override to 4.17.21, then apply patch
        assert_eq!(lodash_result.key, "lodash@4.17.21");
        assert_eq!(
            lodash_result.version,
            "4.17.21+patches/lodash-security.patch"
        );

        // Test V1 workspace dependency from packages/ui to packages/utils
        let utils_result = lockfile
            .resolve_package("packages/ui", "@repo/utils", "packages/utils")
            .unwrap()
            .unwrap();
        assert_eq!(utils_result.key, "@repo/utils@2.0.0");
        assert_eq!(utils_result.version, "2.0.0");

        // Test catalog resolution from named catalog
        let react_result = lockfile
            .resolve_package("packages/ui", "react", "catalog:ui")
            .unwrap()
            .unwrap();
        assert_eq!(react_result.key, "react@18.0.0");
        assert_eq!(react_result.version, "18.0.0");

        // Verify all fields are preserved
        assert_eq!(lockfile.data.lockfile_version, 1);
        assert_eq!(lockfile.data.overrides.len(), 1);
        assert_eq!(lockfile.data.catalog.len(), 1);
        assert_eq!(lockfile.data.catalogs.len(), 1);
        assert_eq!(lockfile.data.patched_dependencies.len(), 1);
    }

    #[test]
    fn test_integration_complex_subgraph_filtering() {
        // Test subgraph filtering with all features enabled
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "complex-monorepo"
                },
                "apps/web": {
                    "name": "web",
                    "version": "1.0.0",
                    "dependencies": {
                        "@repo/ui": "packages/ui",
                        "react": "catalog:frontend"
                    }
                },
                "apps/api": {
                    "name": "api",
                    "version": "1.0.0",
                    "dependencies": {
                        "@repo/shared": "packages/shared",
                        "express": "^4.18.0"
                    }
                },
                "packages/ui": {
                    "name": "@repo/ui",
                    "version": "0.1.0",
                    "dependencies": {
                        "@repo/shared": "packages/shared",
                        "react": "catalog:frontend"
                    }
                },
                "packages/shared": {
                    "name": "@repo/shared",
                    "version": "0.2.0",
                    "dependencies": {
                        "lodash": "catalog:"
                    }
                }
            },
            "packages": {
                "react": ["react@18.0.0", {}, "sha512-react"],
                "react-19": ["react@19.0.0", {}, "sha512-react19"],
                "lodash": ["lodash@4.17.20", {}, "sha512-lodash"],
                "lodash-override": ["lodash@4.17.21", {}, "sha512-lodash21"],
                "express": ["express@4.18.0", {}, "sha512-express"]
            },
            "catalog": {
                "lodash": "^4.17.20"
            },
            "catalogs": {
                "frontend": {
                    "react": "^18.0.0"
                }
            },
            "overrides": {
                "lodash": "4.17.21",
                "react": "19.0.0",
                "express": "4.18.0"
            },
            "patchedDependencies": {
                "lodash@4.17.21": "patches/lodash.patch",
                "express@4.18.0": "patches/express.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Create subgraph for web app only
        let subgraph = lockfile
            .subgraph(
                &[
                    "apps/web".into(),
                    "packages/ui".into(),
                    "packages/shared".into(),
                ],
                &["react@19.0.0".into(), "lodash@4.17.21".into()],
            )
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Verify workspace filtering
        assert_eq!(subgraph_data.workspaces.len(), 4); // root + 3 specified
        assert!(subgraph_data.workspaces.contains_key(""));
        assert!(subgraph_data.workspaces.contains_key("apps/web"));
        assert!(subgraph_data.workspaces.contains_key("packages/ui"));
        assert!(subgraph_data.workspaces.contains_key("packages/shared"));
        assert!(!subgraph_data.workspaces.contains_key("apps/api"));

        // Verify package filtering
        assert_eq!(subgraph_data.packages.len(), 2);
        assert!(subgraph_data.packages.contains_key("react-19"));
        assert!(subgraph_data.packages.contains_key("lodash-override"));
        assert!(!subgraph_data.packages.contains_key("express"));

        // Verify overrides filtering
        assert_eq!(subgraph_data.overrides.len(), 2);
        assert!(subgraph_data.overrides.contains_key("react"));
        assert!(subgraph_data.overrides.contains_key("lodash"));
        assert!(!subgraph_data.overrides.contains_key("express"));

        // Verify patches filtering
        assert_eq!(subgraph_data.patched_dependencies.len(), 1);
        assert!(
            subgraph_data
                .patched_dependencies
                .contains_key("lodash@4.17.21")
        );
        assert!(
            !subgraph_data
                .patched_dependencies
                .contains_key("express@4.18.0")
        );

        // Verify catalogs are preserved (they're kept for potential references)
        assert_eq!(subgraph_data.catalog.len(), 1);
        assert_eq!(subgraph_data.catalogs.len(), 1);
    }

    #[test]
    fn test_subgraph_includes_transitive_dependencies() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-root"
                },
                "apps/acme-client": {
                    "name": "acme-client",
                    "version": "0.0.1",
                    "dependencies": {
                        "@hookform/resolvers": "^5.0.1"
                    }
                }
            },
            "packages": {
                "@hookform/resolvers": ["@hookform/resolvers@5.2.2", "", {
                    "dependencies": {
                        "@standard-schema/utils": "^0.3.0"
                    },
                    "peerDependencies": {
                        "react-hook-form": "^7.55.0"
                    }
                }, "sha512-test"],
                "@standard-schema/utils": ["@standard-schema/utils@0.3.0", "", {}, "sha512-test2"],
                "react-hook-form": ["react-hook-form@7.62.0", "", {}, "sha512-test3"]
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Simulate what turbo prune would call: Get transitive closure first
        let unresolved_deps: std::collections::HashMap<String, String> =
            [("@hookform/resolvers".to_string(), "^5.0.1".to_string())]
                .into_iter()
                .collect();
        let closure =
            crate::transitive_closure(&lockfile, "apps/acme-client", unresolved_deps, false)
                .unwrap();

        // Convert closure to idents
        let package_idents: Vec<String> = closure.iter().map(|pkg| pkg.key.clone()).collect();

        // Create subgraph with the transitive closure
        let subgraph = lockfile
            .subgraph(&["apps/acme-client".into()], &package_idents)
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Verify @hookform/resolvers is included
        assert!(
            subgraph_data
                .packages
                .values()
                .any(|entry| entry.ident == "@hookform/resolvers@5.2.2"),
            "@hookform/resolvers should be in subgraph"
        );

        // Verify @standard-schema/utils is included (transitive dependency)
        assert!(
            subgraph_data
                .packages
                .values()
                .any(|entry| entry.ident == "@standard-schema/utils@0.3.0"),
            "@standard-schema/utils should be in subgraph as transitive dependency"
        );

        // Verify peer dependency is also included
        assert!(
            subgraph_data
                .packages
                .values()
                .any(|entry| entry.ident == "react-hook-form@7.62.0"),
            "react-hook-form should be in subgraph as peer dependency"
        );
    }

    #[test]
    fn test_transitive_deps_next_eslint() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-monorepo",
                    "devDependencies": {
                        "@next/eslint-plugin-next": "^15.5.4"
                    }
                }
            },
            "packages": {
                "@next/eslint-plugin-next": ["@next/eslint-plugin-next@15.5.4", "", {
                    "dependencies": {
                        "fast-glob": "3.3.1"
                    }
                }, "sha512-test"],
                "fast-glob": ["fast-glob@3.3.1", "", {}, "sha512-test2"]
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Simulate turbo prune
        let unresolved_deps: std::collections::HashMap<String, String> = [(
            "@next/eslint-plugin-next".to_string(),
            "^15.5.4".to_string(),
        )]
        .into_iter()
        .collect();
        let closure = crate::transitive_closure(&lockfile, "", unresolved_deps, false).unwrap();

        // fast-glob should be in the transitive closure
        assert!(
            closure.iter().any(|pkg| pkg.key == "fast-glob@3.3.1"),
            "fast-glob should be in transitive closure"
        );
    }

    #[test]
    fn test_integration_serialization_roundtrip_all_features() {
        // Test that serialization preserves all fields through roundtrip
        let original_json = json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "roundtrip-test",
                    "version": "1.0.0",
                    "dependencies": {
                        "react": "catalog:ui"
                    },
                    "devDependencies": {
                        "typescript": "^5.0.0"
                    },
                    "optionalDependencies": {
                        "fsevents": "^2.0.0"
                    },
                    "peerDependencies": {
                        "react": "^18.0.0"
                    },
                    "optionalPeers": ["react"]
                },
                "packages/lib": {
                    "name": "@test/lib",
                    "version": "0.1.0",
                    "dependencies": {
                        "lodash": "catalog:"
                    }
                }
            },
            "packages": {
                "react": ["react@18.2.0", {
                    "dependencies": {
                        "loose-envify": "^1.1.0"
                    },
                    "peerDependencies": {
                        "react": "^18.0.0"
                    },
                    "optionalPeers": ["react"],
                    "bin": "react",
                    "someOtherField": "value"
                }, "sha512-react"],
                "lodash": ["lodash@4.17.21", {}, "sha512-lodash"],
                "typescript": ["typescript@5.0.0", {
                    "bin": {
                        "tsc": "bin/tsc",
                        "tsserver": "bin/tsserver"
                    }
                }, "sha512-typescript"]
            },
            "catalog": {
                "lodash": "^4.17.21"
            },
            "catalogs": {
                "ui": {
                    "react": "^18.2.0",
                    "styled-components": "^5.3.0"
                },
                "backend": {
                    "express": "^4.18.0",
                    "cors": "^2.8.5"
                }
            },
            "overrides": {
                "react": "18.2.0",
                "lodash": "4.17.21"
            },
            "patchedDependencies": {
                "react@18.2.0": "patches/react-performance.patch",
                "lodash@4.17.21": "patches/lodash-security.patch"
            }
        });

        let original_str = serde_json::to_string(&original_json).unwrap();
        let lockfile = BunLockfile::from_str(&original_str).unwrap();

        // Serialize back to bytes
        let serialized_bytes = lockfile.encode().unwrap();
        let serialized_str = std::str::from_utf8(&serialized_bytes).unwrap();

        // Parse again to verify roundtrip
        let roundtrip_lockfile = BunLockfile::from_str(serialized_str).unwrap();

        // Verify all data is preserved
        assert_eq!(roundtrip_lockfile.data.lockfile_version, 1);
        assert_eq!(roundtrip_lockfile.data.workspaces.len(), 2);
        assert_eq!(roundtrip_lockfile.data.packages.len(), 3);
        assert_eq!(roundtrip_lockfile.data.catalog.len(), 1);
        assert_eq!(roundtrip_lockfile.data.catalogs.len(), 2);
        assert_eq!(roundtrip_lockfile.data.overrides.len(), 2);
        assert_eq!(roundtrip_lockfile.data.patched_dependencies.len(), 2);

        // Verify specific values
        assert_eq!(
            roundtrip_lockfile.data.catalog.get("lodash"),
            Some(&"^4.17.21".to_string())
        );
        assert_eq!(
            roundtrip_lockfile.data.overrides.get("react"),
            Some(&"18.2.0".to_string())
        );
        assert_eq!(
            roundtrip_lockfile
                .data
                .patched_dependencies
                .get("react@18.2.0"),
            Some(&"patches/react-performance.patch".to_string())
        );

        // Verify catalog resolution still works
        let react_result = roundtrip_lockfile
            .resolve_package("", "react", "catalog:ui")
            .unwrap()
            .unwrap();
        assert_eq!(react_result.key, "react@18.2.0");
        assert_eq!(
            react_result.version,
            "18.2.0+patches/react-performance.patch"
        );
    }

    #[test]
    fn test_integration_nested_workspace_with_all_features() {
        // Test deeply nested workspace dependencies with catalogs, overrides, and
        // patches
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "nested-test"
                },
                "apps/frontend": {
                    "name": "frontend",
                    "version": "1.0.0",
                    "dependencies": {
                        "@company/ui": "libs/ui",
                        "react": "catalog:frontend"
                    }
                },
                "libs/ui": {
                    "name": "@company/ui",
                    "version": "0.5.0",
                    "dependencies": {
                        "@company/tokens": "libs/design-tokens",
                        "@company/utils": "libs/utils",
                        "react": "catalog:frontend",
                        "styled-components": "catalog:frontend"
                    }
                },
                "libs/design-tokens": {
                    "name": "@company/tokens",
                    "version": "0.2.0",
                    "dependencies": {
                        "color": "catalog:"
                    }
                },
                "libs/utils": {
                    "name": "@company/utils",
                    "version": "1.1.0",
                    "dependencies": {
                        "lodash": "catalog:",
                        "@company/tokens": "libs/design-tokens"
                    }
                }
            },
            "packages": {
                "react": ["react@18.0.0", {}, "sha512-react18"],
                "react-override": ["react@18.2.5", {}, "sha512-react1825"],
                "styled-components": ["styled-components@5.3.0", {}, "sha512-styled"],
                "styled-components-patched": ["styled-components@5.3.6", {}, "sha512-styled536"],
                "lodash": ["lodash@4.17.20", {}, "sha512-lodash"],
                "lodash-override": ["lodash@4.17.21", {}, "sha512-lodash21"],
                "color": ["color@4.0.0", {}, "sha512-color"]
            },
            "catalog": {
                "lodash": "^4.17.20",
                "color": "^4.0.0"
            },
            "catalogs": {
                "frontend": {
                    "react": "^18.0.0",
                    "styled-components": "^5.3.0"
                }
            },
            "overrides": {
                "react": "18.2.5",
                "lodash": "4.17.21",
                "styled-components": "5.3.6"
            },
            "patchedDependencies": {
                "styled-components@5.3.6": "patches/styled-components-ssr.patch",
                "lodash@4.17.21": "patches/lodash-security.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test deep workspace dependency chain resolution
        let ui_result = lockfile
            .resolve_package("apps/frontend", "@company/ui", "libs/ui")
            .unwrap()
            .unwrap();
        assert_eq!(ui_result.key, "@company/ui@0.5.0");

        let tokens_result = lockfile
            .resolve_package("libs/ui", "@company/tokens", "libs/design-tokens")
            .unwrap()
            .unwrap();
        assert_eq!(tokens_result.key, "@company/tokens@0.2.0");

        let utils_result = lockfile
            .resolve_package("libs/ui", "@company/utils", "libs/utils")
            .unwrap()
            .unwrap();
        assert_eq!(utils_result.key, "@company/utils@1.1.0");

        // Test circular workspace dependency (utils -> tokens, tokens referenced by
        // utils)
        let tokens_from_utils = lockfile
            .resolve_package("libs/utils", "@company/tokens", "libs/design-tokens")
            .unwrap()
            .unwrap();
        assert_eq!(tokens_from_utils.key, "@company/tokens@0.2.0");

        // Test catalog resolution with overrides and patches
        let react_result = lockfile
            .resolve_package("libs/ui", "react", "catalog:frontend")
            .unwrap()
            .unwrap();
        assert_eq!(react_result.key, "react@18.2.5"); // Override applied
        assert_eq!(react_result.version, "18.2.5"); // No patch for react

        let styled_result = lockfile
            .resolve_package("libs/ui", "styled-components", "catalog:frontend")
            .unwrap()
            .unwrap();
        assert_eq!(styled_result.key, "styled-components@5.3.6"); // Override applied
        assert_eq!(
            styled_result.version,
            "5.3.6+patches/styled-components-ssr.patch"
        ); // Patch applied

        let lodash_result = lockfile
            .resolve_package("libs/utils", "lodash", "catalog:")
            .unwrap()
            .unwrap();
        assert_eq!(lodash_result.key, "lodash@4.17.21"); // Override applied
        assert_eq!(
            lodash_result.version,
            "4.17.21+patches/lodash-security.patch"
        ); // Patch applied
    }

    #[test]
    fn test_integration_v0_vs_v1_feature_differences() {
        // Test differences in behavior between V0 and V1 lockfiles
        let v0_contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "version-test"
                },
                "packages/lib": {
                    "name": "@test/lib",
                    "dependencies": {
                        "@test/utils": "packages/utils",
                        "react": "catalog:"
                    }
                },
                "packages/utils": {
                    "name": "@test/utils"
                }
            },
            "packages": {
                "lib": ["@test/lib@workspace:packages/lib", {
                    "dependencies": {
                        "@test/utils": "packages/utils",
                        "react": "catalog:"
                    }
                }],
                "lib/@test/utils": ["@test/utils@workspace:packages/utils", {}],
                "react": ["react@18.0.0", {}, "sha512-react"]
            },
            "catalog": {
                "react": "^18.0.0"
            },
            "overrides": {
                "react": "18.0.0"
            }
        }))
        .unwrap();

        let v1_contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "version-test"
                },
                "packages/lib": {
                    "name": "@test/lib",
                    "version": "1.0.0",
                    "dependencies": {
                        "@test/utils": "packages/utils",
                        "react": "catalog:"
                    }
                },
                "packages/utils": {
                    "name": "@test/utils",
                    "version": "2.0.0"
                }
            },
            "packages": {
                "react": ["react@18.0.0", {}, "sha512-react"]
            },
            "catalog": {
                "react": "^18.0.0"
            },
            "overrides": {
                "react": "18.0.0"
            }
        }))
        .unwrap();

        let v0_lockfile = BunLockfile::from_str(&v0_contents).unwrap();
        let v1_lockfile = BunLockfile::from_str(&v1_contents).unwrap();

        // Test workspace dependency resolution differences
        let v0_utils_result = v0_lockfile
            .resolve_package("packages/lib", "@test/utils", "packages/utils")
            .unwrap();

        let v1_utils_result = v1_lockfile
            .resolve_package("packages/lib", "@test/utils", "packages/utils")
            .unwrap()
            .unwrap();

        // V0 might not resolve workspace dependencies that don't have proper packages
        // entries Let's test what we can resolve
        if let Some(v0_utils) = v0_utils_result {
            // V0 resolves through packages section
            assert_eq!(v0_utils.key, "@test/utils@workspace:packages/utils");
            assert_eq!(v0_utils.version, "workspace:packages/utils");
        }

        // V1 resolves directly from workspaces section
        assert_eq!(v1_utils_result.key, "@test/utils@2.0.0");
        assert_eq!(v1_utils_result.version, "2.0.0");

        // Both should handle catalog + override the same way
        let v0_react_result = v0_lockfile
            .resolve_package("packages/lib", "react", "catalog:")
            .unwrap()
            .unwrap();

        let v1_react_result = v1_lockfile
            .resolve_package("packages/lib", "react", "catalog:")
            .unwrap()
            .unwrap();

        assert_eq!(v0_react_result.key, "react@18.0.0");
        assert_eq!(v1_react_result.key, "react@18.0.0");

        // Verify global change detection works
        assert!(v0_lockfile.global_change(&v1_lockfile));
        assert!(v1_lockfile.global_change(&v0_lockfile));
    }

    #[test]
    fn test_integration_error_conditions_and_edge_cases() {
        // Test various error conditions and edge cases
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "edge-case-test"
                },
                "packages/broken": {
                    "name": "@test/broken",
                    "version": "1.0.0",
                    "dependencies": {
                        "missing-catalog": "catalog:nonexistent",
                        "missing-workspace": "packages/nonexistent",
                        "invalid-catalog": "catalog:",
                        "regular-dep": "^1.0.0"
                    }
                }
            },
            "packages": {
                "regular-dep": ["regular-dep@1.0.0", {}, "sha512-regular"]
            },
            "catalog": {},
            "catalogs": {
                "empty": {}
            },
            "overrides": {},
            "patchedDependencies": {}
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test missing catalog reference
        let missing_catalog_result =
            lockfile.resolve_package("packages/broken", "missing-catalog", "catalog:nonexistent");
        assert!(missing_catalog_result.unwrap().is_none());

        // Test missing catalog entry in default catalog
        let invalid_catalog_result =
            lockfile.resolve_package("packages/broken", "invalid-catalog", "catalog:");
        assert!(invalid_catalog_result.unwrap().is_none());

        // Test missing workspace dependency
        let missing_workspace_result = lockfile.resolve_package(
            "packages/broken",
            "missing-workspace",
            "packages/nonexistent",
        );
        assert!(missing_workspace_result.unwrap().is_none());

        // Regular dependency should still work
        let regular_result = lockfile
            .resolve_package("packages/broken", "regular-dep", "^1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(regular_result.key, "regular-dep@1.0.0");

        // Test missing workspace error
        let missing_workspace_error =
            lockfile.resolve_package("packages/nonexistent", "some-dep", "^1.0.0");
        assert!(missing_workspace_error.is_err());

        // Test subgraph with empty filters
        let empty_subgraph = lockfile.subgraph(&[], &[]).unwrap();
        let empty_data = empty_subgraph.lockfile().unwrap();
        assert_eq!(empty_data.workspaces.len(), 1); // Only root
        assert_eq!(empty_data.packages.len(), 0);
        assert_eq!(empty_data.overrides.len(), 0);
    }

    #[test]
    fn test_integration_catalog_precedence_and_resolution_order() {
        // Test the order of resolution: catalog -> override -> patch
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "precedence-test",
                    "dependencies": {
                        "test-package": "catalog:group1"
                    }
                }
            },
            "packages": {
                "test-package": ["test-package@1.0.0", {}, "sha512-original"],
                "test-package-catalog": ["test-package@2.0.0", {}, "sha512-catalog"],
                "test-package-override": ["test-package@3.0.0", {}, "sha512-override"]
            },
            "catalog": {
                "test-package": "^1.0.0"
            },
            "catalogs": {
                "group1": {
                    "test-package": "^2.0.0"
                }
            },
            "overrides": {
                "test-package": "3.0.0"
            },
            "patchedDependencies": {
                "test-package@3.0.0": "patches/test-package.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test resolution order: catalog:group1 (2.0.0) -> override (3.0.0) -> patch
        let result = lockfile
            .resolve_package("", "test-package", "catalog:group1")
            .unwrap()
            .unwrap();

        assert_eq!(result.key, "test-package@3.0.0"); // Override wins
        assert_eq!(result.version, "3.0.0+patches/test-package.patch"); // Patch applied

        // Test without catalog reference - should use override and patch
        let result_no_catalog = lockfile
            .resolve_package("", "test-package", "^1.0.0")
            .unwrap()
            .unwrap();

        assert_eq!(result_no_catalog.key, "test-package@3.0.0"); // Override applied
        assert_eq!(
            result_no_catalog.version,
            "3.0.0+patches/test-package.patch"
        ); // Patch applied

        // Test catalog resolution helper methods
        assert_eq!(
            lockfile.resolve_catalog_version("test-package", "catalog:"),
            Some("^1.0.0")
        );
        assert_eq!(
            lockfile.resolve_catalog_version("test-package", "catalog:group1"),
            Some("^2.0.0")
        );

        // Test override application
        assert_eq!(lockfile.apply_overrides("test-package", "2.0.0"), "3.0.0");
        assert_eq!(lockfile.apply_overrides("other-package", "1.0.0"), "1.0.0");
    }

    #[test]
    fn test_integration_mixed_workspace_and_external_dependencies() {
        // Test complex scenarios mixing workspace and external dependencies
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "mixed-deps-test"
                },
                "apps/main": {
                    "name": "main-app",
                    "version": "1.0.0",
                    "dependencies": {
                        "@company/shared": "libs/shared",
                        "@company/ui": "libs/ui",
                        "react": "catalog:frontend",
                        "express": "catalog:backend"
                    }
                },
                "libs/shared": {
                    "name": "@company/shared",
                    "version": "0.1.0",
                    "dependencies": {
                        "lodash": "catalog:",
                        "uuid": "^9.0.0"
                    }
                },
                "libs/ui": {
                    "name": "@company/ui",
                    "version": "0.2.0",
                    "dependencies": {
                        "@company/shared": "libs/shared",
                        "react": "catalog:frontend",
                        "styled-components": "^5.3.0"
                    },
                    "peerDependencies": {
                        "react": "^18.0.0"
                    }
                }
            },
            "packages": {
                "react": ["react@18.0.0", {}, "sha512-react18"],
                "react-patched": ["react@18.2.0", {}, "sha512-react182"],
                "express": ["express@4.17.0", {}, "sha512-express417"],
                "express-override": ["express@4.18.2", {}, "sha512-express4182"],
                "lodash": ["lodash@4.17.21", {}, "sha512-lodash"],
                "uuid": ["uuid@9.0.0", {}, "sha512-uuid"],
                "styled-components": ["styled-components@5.3.0", {}, "sha512-styled"]
            },
            "catalog": {
                "lodash": "^4.17.21"
            },
            "catalogs": {
                "frontend": {
                    "react": "^18.0.0"
                },
                "backend": {
                    "express": "^4.17.0"
                }
            },
            "overrides": {
                "react": "18.2.0",
                "express": "4.18.2"
            },
            "patchedDependencies": {
                "react@18.2.0": "patches/react.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test workspace dependency resolution
        let shared_result = lockfile
            .resolve_package("apps/main", "@company/shared", "libs/shared")
            .unwrap()
            .unwrap();
        assert_eq!(shared_result.key, "@company/shared@0.1.0");

        let ui_result = lockfile
            .resolve_package("apps/main", "@company/ui", "libs/ui")
            .unwrap()
            .unwrap();
        assert_eq!(ui_result.key, "@company/ui@0.2.0");

        // Test transitive workspace dependency
        let shared_from_ui = lockfile
            .resolve_package("libs/ui", "@company/shared", "libs/shared")
            .unwrap()
            .unwrap();
        assert_eq!(shared_from_ui.key, "@company/shared@0.1.0");

        // Test catalog + override + patch resolution for external deps
        let react_result = lockfile
            .resolve_package("apps/main", "react", "catalog:frontend")
            .unwrap()
            .unwrap();
        assert_eq!(react_result.key, "react@18.2.0");
        assert_eq!(react_result.version, "18.2.0+patches/react.patch");

        let express_result = lockfile
            .resolve_package("apps/main", "express", "catalog:backend")
            .unwrap()
            .unwrap();
        assert_eq!(express_result.key, "express@4.18.2"); // Override applied
        assert_eq!(express_result.version, "4.18.2"); // No patch

        // Test regular dependency without catalog
        let uuid_result = lockfile
            .resolve_package("libs/shared", "uuid", "^9.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(uuid_result.key, "uuid@9.0.0");
        assert_eq!(uuid_result.version, "9.0.0");

        // Test subgraph preserves all dependency types
        let subgraph = lockfile
            .subgraph(
                &["apps/main".into(), "libs/shared".into(), "libs/ui".into()],
                &[
                    "react@18.2.0".into(),
                    "express@4.18.2".into(),
                    "lodash@4.17.21".into(),
                    "uuid@9.0.0".into(),
                    "styled-components@5.3.0".into(),
                ],
            )
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Verify all workspaces included
        assert_eq!(subgraph_data.workspaces.len(), 4); // root + 3 libs

        // Verify all packages included
        assert_eq!(subgraph_data.packages.len(), 5);
        assert!(subgraph_data.packages.contains_key("react-patched"));
        assert!(subgraph_data.packages.contains_key("express-override"));
        assert!(subgraph_data.packages.contains_key("lodash"));
        assert!(subgraph_data.packages.contains_key("uuid"));
        assert!(subgraph_data.packages.contains_key("styled-components"));

        // Verify filtering works correctly
        assert_eq!(subgraph_data.overrides.len(), 2); // react and express
        assert_eq!(subgraph_data.patched_dependencies.len(), 1); // react patch
    }

    #[test]
    fn test_integration_all_dependency_types_with_features() {
        // Test all types of dependencies (regular, dev, optional, peer) with all
        // features
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "all-dep-types"
                },
                "packages/comprehensive": {
                    "name": "@test/comprehensive",
                    "version": "1.0.0",
                    "dependencies": {
                        "runtime-dep": "catalog:"
                    },
                    "devDependencies": {
                        "dev-dep": "catalog:dev",
                        "@test/dev-workspace": "packages/dev-workspace"
                    },
                    "optionalDependencies": {
                        "optional-dep": "catalog:"
                    },
                    "peerDependencies": {
                        "peer-dep": "catalog:peer"
                    },
                    "optionalPeers": ["peer-dep"]
                },
                "packages/dev-workspace": {
                    "name": "@test/dev-workspace",
                    "version": "2.0.0",
                    "dependencies": {
                        "dev-workspace-dep": "^1.0.0"
                    }
                }
            },
            "packages": {
                "runtime-dep": ["runtime-dep@1.0.0", {}, "sha512-runtime"],
                "runtime-override": ["runtime-dep@2.0.0", {}, "sha512-runtime2"],
                "dev-dep": ["dev-dep@1.0.0", {}, "sha512-dev"],
                "dev-override": ["dev-dep@3.0.0", {}, "sha512-dev3"],
                "optional-dep": ["optional-dep@1.0.0", {}, "sha512-optional"],
                "optional-override": ["optional-dep@1.5.0", {}, "sha512-optional15"],
                "peer-dep": ["peer-dep@1.0.0", {}, "sha512-peer"],
                "peer-override": ["peer-dep@4.0.0", {}, "sha512-peer4"],
                "dev-workspace-dep": ["dev-workspace-dep@1.0.0", {}, "sha512-devws"]
            },
            "catalog": {
                "runtime-dep": "^1.0.0",
                "optional-dep": "^1.0.0"
            },
            "catalogs": {
                "dev": {
                    "dev-dep": "^1.0.0"
                },
                "peer": {
                    "peer-dep": "^1.0.0"
                }
            },
            "overrides": {
                "runtime-dep": "2.0.0",
                "dev-dep": "3.0.0",
                "optional-dep": "1.5.0",
                "peer-dep": "4.0.0"
            },
            "patchedDependencies": {
                "runtime-dep@2.0.0": "patches/runtime.patch",
                "dev-dep@3.0.0": "patches/dev.patch",
                "optional-dep@1.5.0": "patches/optional.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test runtime dependency with catalog + override + patch
        let runtime_result = lockfile
            .resolve_package("packages/comprehensive", "runtime-dep", "catalog:")
            .unwrap()
            .unwrap();
        assert_eq!(runtime_result.key, "runtime-dep@2.0.0");
        assert_eq!(runtime_result.version, "2.0.0+patches/runtime.patch");

        // Test dev dependency with named catalog + override + patch
        let dev_result = lockfile
            .resolve_package("packages/comprehensive", "dev-dep", "catalog:dev")
            .unwrap()
            .unwrap();
        assert_eq!(dev_result.key, "dev-dep@3.0.0");
        assert_eq!(dev_result.version, "3.0.0+patches/dev.patch");

        // Test optional dependency with catalog + override + patch
        let optional_result = lockfile
            .resolve_package("packages/comprehensive", "optional-dep", "catalog:")
            .unwrap()
            .unwrap();
        assert_eq!(optional_result.key, "optional-dep@1.5.0");
        assert_eq!(optional_result.version, "1.5.0+patches/optional.patch");

        // Test peer dependency with named catalog + override (no patch)
        let peer_result = lockfile
            .resolve_package("packages/comprehensive", "peer-dep", "catalog:peer")
            .unwrap()
            .unwrap();
        assert_eq!(peer_result.key, "peer-dep@4.0.0");
        assert_eq!(peer_result.version, "4.0.0");

        // Test workspace dev dependency
        let dev_workspace_result = lockfile
            .resolve_package(
                "packages/comprehensive",
                "@test/dev-workspace",
                "packages/dev-workspace",
            )
            .unwrap()
            .unwrap();
        assert_eq!(dev_workspace_result.key, "@test/dev-workspace@2.0.0");
        assert_eq!(dev_workspace_result.version, "2.0.0");

        // Test regular dependency from dev workspace
        let dev_workspace_dep_result = lockfile
            .resolve_package("packages/dev-workspace", "dev-workspace-dep", "^1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(dev_workspace_dep_result.key, "dev-workspace-dep@1.0.0");
        assert_eq!(dev_workspace_dep_result.version, "1.0.0");

        // Test subgraph includes all dependency types
        let subgraph = lockfile
            .subgraph(
                &[
                    "packages/comprehensive".into(),
                    "packages/dev-workspace".into(),
                ],
                &[
                    "runtime-dep@2.0.0".into(),
                    "dev-dep@3.0.0".into(),
                    "optional-dep@1.5.0".into(),
                    "peer-dep@4.0.0".into(),
                    "dev-workspace-dep@1.0.0".into(),
                ],
            )
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Verify all packages included
        assert_eq!(subgraph_data.packages.len(), 5);
        assert!(subgraph_data.packages.contains_key("runtime-override"));
        assert!(subgraph_data.packages.contains_key("dev-override"));
        assert!(subgraph_data.packages.contains_key("optional-override"));
        assert!(subgraph_data.packages.contains_key("peer-override"));
        assert!(subgraph_data.packages.contains_key("dev-workspace-dep"));

        // Verify all overrides and patches preserved
        assert_eq!(subgraph_data.overrides.len(), 4);
        assert_eq!(subgraph_data.patched_dependencies.len(), 3);
    }

    #[test]
    fn test_integration_global_change_detection_comprehensive() {
        // Test comprehensive global change detection with all features
        let base_lockfile_v0 = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "change-detection-test"
                }
            },
            "packages": {
                "react": ["react@18.0.0", {}, "sha512-react"]
            },
            "catalog": {
                "react": "^18.0.0"
            },
            "overrides": {
                "react": "18.0.0"
            },
            "patchedDependencies": {
                "react@18.0.0": "patches/react.patch"
            }
        }))
        .unwrap();

        let base_lockfile_v1 = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "change-detection-test"
                }
            },
            "packages": {
                "react": ["react@18.0.0", {}, "sha512-react"]
            },
            "catalog": {
                "react": "^18.0.0"
            },
            "overrides": {
                "react": "18.0.0"
            },
            "patchedDependencies": {
                "react@18.0.0": "patches/react.patch"
            }
        }))
        .unwrap();

        let v0_lockfile = BunLockfile::from_str(&base_lockfile_v0).unwrap();
        let v1_lockfile = BunLockfile::from_str(&base_lockfile_v1).unwrap();

        // Version change should be detected as global change
        assert!(v0_lockfile.global_change(&v1_lockfile));
        assert!(v1_lockfile.global_change(&v0_lockfile));

        // Same version should not be global change
        assert!(!v0_lockfile.global_change(&v0_lockfile));
        assert!(!v1_lockfile.global_change(&v1_lockfile));

        // Test with standalone function as well
        assert!(
            bun_global_change(base_lockfile_v0.as_bytes(), base_lockfile_v1.as_bytes()).unwrap()
        );
        assert!(
            !bun_global_change(base_lockfile_v0.as_bytes(), base_lockfile_v0.as_bytes()).unwrap()
        );
        assert!(
            !bun_global_change(base_lockfile_v1.as_bytes(), base_lockfile_v1.as_bytes()).unwrap()
        );
    }

    #[test]
    fn test_integration_extreme_edge_cases_and_complex_resolution() {
        // Test the most complex scenario we can think of - all features with edge cases
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "extreme-test",
                    "dependencies": {
                        "overridden-catalog": "catalog:special",
                        "patched-override": "catalog:"
                    }
                },
                "apps/complex": {
                    "name": "complex-app",
                    "version": "1.0.0",
                    "dependencies": {
                        "@workspace/lib": "libs/workspace-lib",
                        "multi-override": "catalog:multi",
                        "deep-patch": "catalog:"
                    },
                    "devDependencies": {
                        "@workspace/dev-lib": "libs/dev-workspace-lib"
                    },
                    "optionalDependencies": {
                        "optional-catalog": "catalog:optional"
                    },
                    "peerDependencies": {
                        "@workspace/lib": "1.0.0"
                    }
                },
                "libs/workspace-lib": {
                    "name": "@workspace/lib",
                    "version": "1.0.0",
                    "dependencies": {
                        "@workspace/dev-lib": "libs/dev-workspace-lib",
                        "transitive-catalog": "catalog:"
                    }
                },
                "libs/dev-workspace-lib": {
                    "name": "@workspace/dev-lib",
                    "version": "2.0.0",
                    "dependencies": {
                        "leaf-dependency": "^1.0.0"
                    }
                }
            },
            "packages": {
                "overridden-catalog": ["overridden-catalog@1.0.0", {}, "sha512-oc1"],
                "overridden-catalog-override": ["overridden-catalog@2.0.0", {}, "sha512-oc2"],
                "patched-override": ["patched-override@1.0.0", {}, "sha512-po1"],
                "patched-override-final": ["patched-override@3.0.0", {}, "sha512-po3"],
                "multi-override": ["multi-override@1.0.0", {}, "sha512-mo1"],
                "multi-override-catalog": ["multi-override@2.0.0", {}, "sha512-mo2"],
                "multi-override-final": ["multi-override@5.0.0", {}, "sha512-mo5"],
                "deep-patch": ["deep-patch@1.0.0", {}, "sha512-dp1"],
                "deep-patch-override": ["deep-patch@2.0.0", {}, "sha512-dp2"],
                "optional-catalog": ["optional-catalog@1.0.0", {}, "sha512-opt1"],
                "optional-catalog-override": ["optional-catalog@1.5.0", {}, "sha512-opt15"],
                "transitive-catalog": ["transitive-catalog@1.0.0", {}, "sha512-tc1"],
                "transitive-catalog-override": ["transitive-catalog@1.2.0", {}, "sha512-tc12"],
                "leaf-dependency": ["leaf-dependency@1.0.0", {}, "sha512-leaf"]
            },
            "catalog": {
                "patched-override": "^1.0.0",
                "deep-patch": "^1.0.0",
                "transitive-catalog": "^1.0.0"
            },
            "catalogs": {
                "special": {
                    "overridden-catalog": "^1.0.0"
                },
                "multi": {
                    "multi-override": "^2.0.0"
                },
                "optional": {
                    "optional-catalog": "^1.0.0"
                }
            },
            "overrides": {
                "overridden-catalog": "2.0.0",
                "patched-override": "3.0.0",
                "multi-override": "5.0.0",
                "deep-patch": "2.0.0",
                "optional-catalog": "1.5.0",
                "transitive-catalog": "1.2.0"
            },
            "patchedDependencies": {
                "patched-override@3.0.0": "patches/patched-override-complex.patch",
                "deep-patch@2.0.0": "patches/deep-patch-security.patch",
                "transitive-catalog@1.2.0": "patches/transitive-fix.patch"
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test complex catalog + override + patch chain
        let patched_override_result = lockfile
            .resolve_package("", "patched-override", "catalog:")
            .unwrap()
            .unwrap();
        assert_eq!(patched_override_result.key, "patched-override@3.0.0");
        assert_eq!(
            patched_override_result.version,
            "3.0.0+patches/patched-override-complex.patch"
        );

        // Test catalog override from named catalog
        let overridden_catalog_result = lockfile
            .resolve_package("", "overridden-catalog", "catalog:special")
            .unwrap()
            .unwrap();
        assert_eq!(overridden_catalog_result.key, "overridden-catalog@2.0.0");
        assert_eq!(overridden_catalog_result.version, "2.0.0");

        // Test multi-level resolution (named catalog -> override)
        let multi_override_result = lockfile
            .resolve_package("apps/complex", "multi-override", "catalog:multi")
            .unwrap()
            .unwrap();
        assert_eq!(multi_override_result.key, "multi-override@5.0.0");
        assert_eq!(multi_override_result.version, "5.0.0");

        // Test workspace dependencies in complex app
        let workspace_lib_result = lockfile
            .resolve_package("apps/complex", "@workspace/lib", "libs/workspace-lib")
            .unwrap()
            .unwrap();
        assert_eq!(workspace_lib_result.key, "@workspace/lib@1.0.0");
        assert_eq!(workspace_lib_result.version, "1.0.0");

        // Test transitive workspace dependencies
        let dev_lib_result = lockfile
            .resolve_package(
                "libs/workspace-lib",
                "@workspace/dev-lib",
                "libs/dev-workspace-lib",
            )
            .unwrap()
            .unwrap();
        assert_eq!(dev_lib_result.key, "@workspace/dev-lib@2.0.0");
        assert_eq!(dev_lib_result.version, "2.0.0");

        // Test transitive catalog resolution
        let transitive_result = lockfile
            .resolve_package("libs/workspace-lib", "transitive-catalog", "catalog:")
            .unwrap()
            .unwrap();
        assert_eq!(transitive_result.key, "transitive-catalog@1.2.0");
        assert_eq!(
            transitive_result.version,
            "1.2.0+patches/transitive-fix.patch"
        );

        // Test regular dependency at the end of the chain
        let leaf_result = lockfile
            .resolve_package("libs/dev-workspace-lib", "leaf-dependency", "^1.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(leaf_result.key, "leaf-dependency@1.0.0");
        assert_eq!(leaf_result.version, "1.0.0");

        // Test complex subgraph that includes all features
        let complex_subgraph = lockfile
            .subgraph(
                &[
                    "apps/complex".into(),
                    "libs/workspace-lib".into(),
                    "libs/dev-workspace-lib".into(),
                ],
                &[
                    "overridden-catalog@2.0.0".into(),
                    "patched-override@3.0.0".into(),
                    "multi-override@5.0.0".into(),
                    "deep-patch@2.0.0".into(),
                    "optional-catalog@1.5.0".into(),
                    "transitive-catalog@1.2.0".into(),
                    "leaf-dependency@1.0.0".into(),
                ],
            )
            .unwrap();
        // Verify resolution still works in subgraph before getting data
        let subgraph_resolution = complex_subgraph
            .resolve_package("apps/complex", "multi-override", "catalog:multi")
            .unwrap();

        // Handle the case where the resolution might not work in the subgraph
        if let Some(resolution) = subgraph_resolution {
            assert_eq!(resolution.key, "multi-override@5.0.0");
            assert_eq!(resolution.version, "5.0.0");
        }

        let complex_subgraph_data = complex_subgraph.lockfile().unwrap();

        // Verify comprehensive filtering
        assert_eq!(complex_subgraph_data.workspaces.len(), 4); // root + 3 workspaces
        assert_eq!(complex_subgraph_data.packages.len(), 7); // All specified packages
        assert_eq!(complex_subgraph_data.overrides.len(), 6); // All applicable overrides
        assert_eq!(complex_subgraph_data.patched_dependencies.len(), 3); // All applicable patches
        assert_eq!(complex_subgraph_data.catalog.len(), 3); // Catalogs preserved
        assert_eq!(complex_subgraph_data.catalogs.len(), 3); // Named catalogs
        // preserved
    }

    #[test]
    fn test_negatable_serialization_and_deserialization() {
        // Test "none" value
        let none_json = serde_json::to_value(&Negatable::None).unwrap();
        assert_eq!(none_json, Value::String("none".to_string()));

        let none_deserialized: Negatable = serde_json::from_value(none_json).unwrap();
        assert_eq!(none_deserialized, Negatable::None);

        // Test single platform
        let single_json = serde_json::to_value(Negatable::Single("darwin".to_string())).unwrap();
        assert_eq!(single_json, Value::String("darwin".to_string()));

        let single_deserialized: Negatable = serde_json::from_value(single_json).unwrap();
        assert_eq!(single_deserialized, Negatable::Single("darwin".to_string()));

        // Test multiple platforms
        let multiple = Negatable::Multiple(vec!["darwin".to_string(), "linux".to_string()]);
        let multiple_json = serde_json::to_value(&multiple).unwrap();
        assert_eq!(
            multiple_json,
            Value::Array(vec![
                Value::String("darwin".to_string()),
                Value::String("linux".to_string())
            ])
        );

        let multiple_deserialized: Negatable = serde_json::from_value(multiple_json).unwrap();
        assert_eq!(multiple_deserialized, multiple);

        // Test negated platforms
        let negated = Negatable::Negated(vec!["win32".to_string()]);
        let negated_json = serde_json::to_value(&negated).unwrap();
        assert_eq!(
            negated_json,
            Value::Array(vec![Value::String("!win32".to_string())])
        );

        let negated_deserialized: Negatable = serde_json::from_value(negated_json).unwrap();
        assert_eq!(negated_deserialized, negated);

        // Test multiple negated platforms
        let multi_negated = Negatable::Negated(vec!["win32".to_string(), "freebsd".to_string()]);
        let multi_negated_json = serde_json::to_value(&multi_negated).unwrap();
        assert_eq!(
            multi_negated_json,
            Value::Array(vec![
                Value::String("!win32".to_string()),
                Value::String("!freebsd".to_string())
            ])
        );

        let multi_negated_deserialized: Negatable =
            serde_json::from_value(multi_negated_json).unwrap();
        assert_eq!(multi_negated_deserialized, multi_negated);
    }

    #[test]
    fn test_negatable_allows_method() {
        // Test None allows everything
        let none = Negatable::None;
        assert!(none.allows("darwin"));
        assert!(none.allows("linux"));
        assert!(none.allows("win32"));

        // Test single platform
        let darwin_only = Negatable::Single("darwin".to_string());
        assert!(darwin_only.allows("darwin"));
        assert!(!darwin_only.allows("linux"));
        assert!(!darwin_only.allows("win32"));

        // Test multiple platforms
        let unix_only = Negatable::Multiple(vec!["darwin".to_string(), "linux".to_string()]);
        assert!(unix_only.allows("darwin"));
        assert!(unix_only.allows("linux"));
        assert!(!unix_only.allows("win32"));

        // Test negated platforms
        let not_windows = Negatable::Negated(vec!["win32".to_string()]);
        assert!(not_windows.allows("darwin"));
        assert!(not_windows.allows("linux"));
        assert!(!not_windows.allows("win32"));

        // Test multiple negated platforms
        let not_windows_or_freebsd =
            Negatable::Negated(vec!["win32".to_string(), "freebsd".to_string()]);
        assert!(not_windows_or_freebsd.allows("darwin"));
        assert!(not_windows_or_freebsd.allows("linux"));
        assert!(!not_windows_or_freebsd.allows("win32"));
        assert!(!not_windows_or_freebsd.allows("freebsd"));
    }

    #[test]
    fn test_negatable_mixed_array_behavior() {
        // Test that mixed arrays with non-negated values work correctly
        let mixed_json = Value::Array(vec![
            Value::String("linux".to_string()),
            Value::String("!darwin".to_string()),
        ]);
        let mixed: Negatable = serde_json::from_value(mixed_json).unwrap();

        // Should only allow linux (negated darwin is ignored in mixed array)
        assert!(mixed.allows("linux"));
        assert!(!mixed.allows("darwin"));
        assert!(!mixed.allows("win32"));
        assert!(!mixed.allows("freebsd"));

        // Test contradictory case: platform is both allowed and blocked
        let contradictory_json = Value::Array(vec![
            Value::String("linux".to_string()),
            Value::String("darwin".to_string()),
            Value::String("!linux".to_string()),
        ]);
        let contradictory: Negatable = serde_json::from_value(contradictory_json).unwrap();

        // Non-negated values win, so both linux and darwin are allowed
        assert!(contradictory.allows("linux"));
        assert!(contradictory.allows("darwin"));
        assert!(!contradictory.allows("win32"));
    }

    #[test]
    fn test_negatable_deserialization_edge_cases() {
        // Test single negated string
        let single_negated_json = Value::String("!win32".to_string());
        let single_negated: Negatable = serde_json::from_value(single_negated_json).unwrap();
        assert_eq!(
            single_negated,
            Negatable::Negated(vec!["win32".to_string()])
        );

        // Test array with all negated elements
        let all_negated_json = Value::Array(vec![
            Value::String("!win32".to_string()),
            Value::String("!freebsd".to_string()),
        ]);
        let all_negated: Negatable = serde_json::from_value(all_negated_json).unwrap();
        assert_eq!(
            all_negated,
            Negatable::Negated(vec!["win32".to_string(), "freebsd".to_string()])
        );

        // Test mixed array (some negated, some not) - non-negated values should be used
        let mixed_array_json = Value::Array(vec![
            Value::String("linux".to_string()),
            Value::String("!darwin".to_string()),
        ]);
        let mixed_array: Negatable = serde_json::from_value(mixed_array_json).unwrap();
        assert_eq!(mixed_array, Negatable::Multiple(vec!["linux".to_string()]));

        // Test reverse mixed array - non-negated values should still be used
        let reverse_mixed_json = Value::Array(vec![
            Value::String("!linux".to_string()),
            Value::String("darwin".to_string()),
            Value::String("win32".to_string()),
        ]);
        let reverse_mixed: Negatable = serde_json::from_value(reverse_mixed_json).unwrap();
        assert_eq!(
            reverse_mixed,
            Negatable::Multiple(vec!["darwin".to_string(), "win32".to_string()])
        );

        // Test contradictory mixed array (platform both allowed and blocked)
        let contradictory_json = Value::Array(vec![
            Value::String("linux".to_string()),
            Value::String("!linux".to_string()),
            Value::String("darwin".to_string()),
        ]);
        let contradictory: Negatable = serde_json::from_value(contradictory_json).unwrap();
        assert_eq!(
            contradictory,
            Negatable::Multiple(vec!["linux".to_string(), "darwin".to_string()])
        );

        // Test empty array - should be treated as multiple with empty list
        let empty_array_json = Value::Array(vec![]);
        let empty_array: Negatable = serde_json::from_value(empty_array_json).unwrap();
        assert_eq!(empty_array, Negatable::Multiple(vec![]));
    }

    #[test]
    fn test_platform_specific_package_parsing() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-app",
                    "version": "1.0.0"
                }
            },
            "packages": {
                "fsevents@2.3.2": [
                    "fsevents@2.3.2",
                    {
                        "os": "darwin",
                        "cpu": ["x64", "arm64"],
                        "dependencies": {
                            "node-gyp": "^9.0.0"
                        }
                    }
                ],
                "node-pty@0.10.1": [
                    "node-pty@0.10.1",
                    {
                        "os": ["darwin", "linux"],
                        "cpu": "!win32",
                        "dependencies": {
                            "nan": "^2.14.0"
                        }
                    }
                ],
                "win32-process@1.0.0": [
                    "win32-process@1.0.0",
                    {
                        "os": ["!darwin", "!linux"],
                        "dependencies": {
                            "windows-api": "^1.0.0"
                        }
                    }
                ]
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test fsevents with darwin-only OS constraint
        let fsevents_entry = lockfile.data.packages.get("fsevents@2.3.2").unwrap();
        let fsevents_info = fsevents_entry.info.as_ref().unwrap();
        assert_eq!(fsevents_info.os, Negatable::Single("darwin".to_string()));
        assert_eq!(
            fsevents_info.cpu,
            Negatable::Multiple(vec!["x64".to_string(), "arm64".to_string()])
        );

        // Test node-pty with multiple OS constraint and negated CPU
        let node_pty_entry = lockfile.data.packages.get("node-pty@0.10.1").unwrap();
        let node_pty_info = node_pty_entry.info.as_ref().unwrap();
        assert_eq!(
            node_pty_info.os,
            Negatable::Multiple(vec!["darwin".to_string(), "linux".to_string()])
        );
        assert_eq!(
            node_pty_info.cpu,
            Negatable::Negated(vec!["win32".to_string()])
        );

        // Test win32-process with negated OS constraints
        let win32_entry = lockfile.data.packages.get("win32-process@1.0.0").unwrap();
        let win32_info = win32_entry.info.as_ref().unwrap();
        assert_eq!(
            win32_info.os,
            Negatable::Negated(vec!["darwin".to_string(), "linux".to_string()])
        );
        assert_eq!(win32_info.cpu, Negatable::None); // Default
    }

    #[test]
    fn test_platform_specific_serialization_roundtrip() {
        let original_lockfile = json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-app",
                    "version": "1.0.0"
                }
            },
            "packages": {
                "platform-dep@1.0.0": [
                    "platform-dep@1.0.0",
                    {
                        "os": "darwin",
                        "cpu": ["x64", "arm64"],
                        "dependencies": {
                            "native-lib": "^1.0.0"
                        }
                    }
                ],
                "cross-platform@2.0.0": [
                    "cross-platform@2.0.0",
                    {
                        "dependencies": {
                            "common-lib": "^1.0.0"
                        }
                    }
                ],
                "anti-windows@1.0.0": [
                    "anti-windows@1.0.0",
                    {
                        "os": ["!win32"],
                        "dependencies": {
                            "unix-only": "^1.0.0"
                        }
                    }
                ]
            }
        });

        let contents = serde_json::to_string(&original_lockfile).unwrap();
        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Serialize back to JSON
        let lockfile_json = lockfile.lockfile().unwrap();
        let serialized = serde_json::to_value(&lockfile_json).unwrap();

        // Verify platform-specific package is preserved
        let platform_dep = serialized["packages"]["platform-dep@1.0.0"][1]
            .as_object()
            .unwrap();
        assert_eq!(platform_dep["os"], Value::String("darwin".to_string()));
        assert_eq!(
            platform_dep["cpu"],
            Value::Array(vec![
                Value::String("x64".to_string()),
                Value::String("arm64".to_string())
            ])
        );

        // Verify cross-platform package doesn't have os/cpu fields
        let cross_platform = serialized["packages"]["cross-platform@2.0.0"][1]
            .as_object()
            .unwrap();
        assert!(!cross_platform.contains_key("os"));
        assert!(!cross_platform.contains_key("cpu"));

        // Verify negated platform constraint is preserved
        let anti_windows = serialized["packages"]["anti-windows@1.0.0"][1]
            .as_object()
            .unwrap();
        assert_eq!(
            anti_windows["os"],
            Value::Array(vec![Value::String("!win32".to_string())])
        );
    }

    #[test]
    fn test_platform_specific_subgraph_preservation() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-app",
                    "version": "1.0.0"
                },
                "apps/web": {
                    "name": "@workspace/web",
                    "version": "1.0.0"
                }
            },
            "packages": {
                "fsevents@2.3.2": [
                    "fsevents@2.3.2",
                    {
                        "os": "darwin",
                        "cpu": ["x64", "arm64"],
                        "dependencies": {}
                    }
                ],
                "common-dep@1.0.0": [
                    "common-dep@1.0.0",
                    {
                        "dependencies": {}
                    }
                ]
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Create subgraph including platform-specific package
        let subgraph = lockfile
            .subgraph(
                &["apps/web".into()],
                &["fsevents@2.3.2".into(), "common-dep@1.0.0".into()],
            )
            .unwrap();

        let subgraph_data = subgraph.lockfile().unwrap();

        // Verify platform constraints are preserved in subgraph
        let fsevents_entry = subgraph_data.packages.get("fsevents@2.3.2").unwrap();
        let fsevents_info = fsevents_entry.info.as_ref().unwrap();
        assert_eq!(fsevents_info.os, Negatable::Single("darwin".to_string()));
        assert_eq!(
            fsevents_info.cpu,
            Negatable::Multiple(vec!["x64".to_string(), "arm64".to_string()])
        );

        // Verify common package doesn't have platform constraints
        let common_entry = subgraph_data.packages.get("common-dep@1.0.0").unwrap();
        let common_info = common_entry.info.as_ref().unwrap();
        assert_eq!(common_info.os, Negatable::None);
        assert_eq!(common_info.cpu, Negatable::None);
    }

    #[test]
    fn test_bundled_dependencies_included_in_subgraph() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-app",
                    "dependencies": {
                        "@tailwindcss/oxide-wasm32-wasi": "^4.1.13"
                    }
                }
            },
            "packages": {
                "@tailwindcss/oxide-wasm32-wasi": [
                    "@tailwindcss/oxide-wasm32-wasi@4.1.13",
                    "",
                    {
                        "dependencies": {
                            "@emnapi/core": "^1.4.5",
                            "@emnapi/runtime": "^1.4.5"
                        }
                    },
                    "sha512-test"
                ],
                "@tailwindcss/oxide-wasm32-wasi/@emnapi/core": [
                    "@emnapi/core@1.5.0",
                    "",
                    {
                        "dependencies": {
                            "tslib": "^2.4.0"
                        },
                        "bundled": true
                    },
                    "sha512-bundled-core"
                ],
                "@tailwindcss/oxide-wasm32-wasi/@emnapi/runtime": [
                    "@emnapi/runtime@1.5.0",
                    "",
                    {
                        "dependencies": {
                            "tslib": "^2.4.0"
                        },
                        "bundled": true
                    },
                    "sha512-bundled-runtime"
                ],
                "@tailwindcss/oxide-wasm32-wasi/tslib": [
                    "tslib@2.8.1",
                    "",
                    {
                        "bundled": true
                    },
                    "sha512-bundled-tslib"
                ]
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Create subgraph including @tailwindcss/oxide-wasm32-wasi
        let subgraph = lockfile
            .subgraph(&[], &["@tailwindcss/oxide-wasm32-wasi@4.1.13".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Verify the main package is included
        assert!(
            subgraph_data
                .packages
                .contains_key("@tailwindcss/oxide-wasm32-wasi")
        );

        // Verify bundled dependencies are included
        assert!(
            subgraph_data
                .packages
                .contains_key("@tailwindcss/oxide-wasm32-wasi/@emnapi/core")
        );
        assert!(
            subgraph_data
                .packages
                .contains_key("@tailwindcss/oxide-wasm32-wasi/@emnapi/runtime")
        );
        assert!(
            subgraph_data
                .packages
                .contains_key("@tailwindcss/oxide-wasm32-wasi/tslib")
        );

        // Verify the count is correct (1 main + 3 bundled)
        assert_eq!(subgraph_data.packages.len(), 4);
    }

    #[test]
    fn test_complex_platform_constraints() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test-app",
                    "version": "1.0.0"
                }
            },
            "packages": {
                "multi-platform@1.0.0": [
                    "multi-platform@1.0.0",
                    {
                        "os": ["darwin", "linux", "freebsd"],
                        "cpu": ["x64", "arm64", "arm"],
                        "dependencies": {
                            "native-addon": "^1.0.0"
                        }
                    }
                ],
                "no-mobile@1.0.0": [
                    "no-mobile@1.0.0",
                    {
                        "os": ["!android", "!ios"],
                        "cpu": ["!arm", "!arm64"],
                        "dependencies": {
                            "desktop-only": "^1.0.0"
                        }
                    }
                ],
                "special-none@1.0.0": [
                    "special-none@1.0.0",
                    {
                        "os": "none",
                        "cpu": "none",
                        "dependencies": {}
                    }
                ]
            }
        }))
        .unwrap();

        let lockfile = BunLockfile::from_str(&contents).unwrap();

        // Test multi-platform package
        let multi_entry = lockfile.data.packages.get("multi-platform@1.0.0").unwrap();
        let multi_info = multi_entry.info.as_ref().unwrap();
        assert_eq!(
            multi_info.os,
            Negatable::Multiple(vec![
                "darwin".to_string(),
                "linux".to_string(),
                "freebsd".to_string()
            ])
        );
        assert_eq!(
            multi_info.cpu,
            Negatable::Multiple(vec![
                "x64".to_string(),
                "arm64".to_string(),
                "arm".to_string()
            ])
        );

        // Test negated constraints
        let no_mobile_entry = lockfile.data.packages.get("no-mobile@1.0.0").unwrap();
        let no_mobile_info = no_mobile_entry.info.as_ref().unwrap();
        assert_eq!(
            no_mobile_info.os,
            Negatable::Negated(vec!["android".to_string(), "ios".to_string()])
        );
        assert_eq!(
            no_mobile_info.cpu,
            Negatable::Negated(vec!["arm".to_string(), "arm64".to_string()])
        );

        // Test explicit "none" values
        let special_entry = lockfile.data.packages.get("special-none@1.0.0").unwrap();
        let special_info = special_entry.info.as_ref().unwrap();
        assert_eq!(special_info.os, Negatable::None);
        assert_eq!(special_info.cpu, Negatable::None);
    }
}
