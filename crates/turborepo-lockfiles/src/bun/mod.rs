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

use std::{any::Any, collections::HashMap, str::FromStr};

use biome_json_formatter::context::JsonFormatOptions;
use biome_json_parser::JsonParserOptions;
use id::PossibleKeyIter;
use itertools::Itertools as _;
use semver::{Version, VersionReq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use turbopath::RelativeUnixPathBuf;
use turborepo_errors::ParseDiagnostic;

use crate::Lockfile;

mod de;
mod id;
mod index;
mod ser;
mod types;

use index::PackageIndex;
pub use types::{PackageIdent, PackageKey, VersionSpec};

type Map<K, V> = std::collections::BTreeMap<K, V>;
type BTreeSet<T> = std::collections::BTreeSet<T>;

/// Represents a platform constraint that can be either inclusive or exclusive.
/// This matches Bun's Negatable type for os/cpu/libc fields.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Negatable {
    /// No constraint - package works on all platforms
    #[default]
    None,
    /// Single platform constraint
    Single(String),
    /// Multiple platform constraints (all must match)
    Multiple(Vec<String>),
    /// Negated constraints - package works on all platforms except these
    Negated(Vec<String>),
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
                // Treating "none" as special breaks wasm packages that use cpu="none"
                if let Some(stripped) = s.strip_prefix('!') {
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
                    // npm spec requires explicit allows to take precedence over denies
                    let allowed_platforms: Vec<String> = platforms
                        .into_iter()
                        .filter(|p| !p.starts_with('!'))
                        .collect();
                    Ok(Negatable::Multiple(allowed_platforms))
                } else if has_negated {
                    // Strip '!' prefix to extract the blocked platforms
                    let negated_platforms: Vec<String> = platforms
                        .into_iter()
                        .map(|p| p.strip_prefix('!').unwrap().to_string())
                        .collect();
                    Ok(Negatable::Negated(negated_platforms))
                } else {
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
    index: PackageIndex,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunLockfileData {
    lockfile_version: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config_version: Option<i32>,
    workspaces: Map<String, WorkspaceEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    trusted_dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    overrides: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    catalog: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    catalogs: Map<String, Map<String, String>>,
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
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    optional_peers: BTreeSet<String>,
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

impl BunLockfile {
    /// Process a package entry, handling workspace filtering, overrides, and
    /// patches
    fn process_package_entry(
        &self,
        entry: &PackageEntry,
        name: &str,
        override_version: &str,
        resolved_version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        let ident = PackageIdent::parse(&entry.ident);

        // Filter out workspace mapping entries
        if ident.is_workspace() {
            return Ok(None);
        }

        // Check for overrides
        if override_version != resolved_version {
            let override_ident = format!("{name}@{override_version}");
            if let Some((_override_key, override_entry)) = self.index.get_by_ident(&override_ident)
            {
                let mut pkg_version = override_entry.version().to_string();
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

        // Return the package with its version (and patch if applicable)
        let mut version = entry.version().to_string();
        if let Some(patch) = self.data.patched_dependencies.get(&entry.ident) {
            version.push('+');
            version.push_str(patch);
        }
        Ok(Some(crate::Package {
            key: entry.ident.to_string(),
            version,
        }))
    }

    /// Check if a package version satisfies a version specification.
    ///
    /// Returns true if the version satisfies the spec, false otherwise.
    /// For non-semver specs (tags, catalogs, workspaces), returns true.
    fn version_satisfies_spec(&self, version: &str, version_spec: &str) -> bool {
        let spec = VersionSpec::parse(version_spec);

        match spec {
            VersionSpec::Semver(spec_str) => {
                // Parse both the requirement and the version
                let Ok(req) = VersionReq::parse(&spec_str) else {
                    // If we can't parse the requirement, be lenient and accept it
                    return true;
                };

                let Ok(ver) = Version::parse(version) else {
                    // If we can't parse the version, be lenient and accept it
                    return true;
                };

                req.matches(&ver)
            }
            // For non-semver specs (tags, catalogs, workspace), accept any version
            // since validation happens elsewhere
            _ => true,
        }
    }

    /// Find a package version that satisfies the given version spec.
    ///
    /// Searches in order:
    /// 1. Workspace-scoped entries
    /// 2. Top-level entries
    /// 3. Nested/aliased entries (by searching all idents)
    fn find_matching_version(
        &self,
        workspace_name: &str,
        name: &str,
        version_spec: &str,
        override_version: &str,
        resolved_version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        // Try workspace-scoped first
        if let Some(entry) = self.index.get_workspace_scoped(workspace_name, name)
            && let Some(pkg) =
                self.process_package_entry(entry, name, override_version, resolved_version)?
            && self.version_satisfies_spec(&pkg.version, version_spec)
        {
            return Ok(Some(pkg));
        }

        // Try hoisted/top-level
        if let Some((_key, entry)) = self.index.find_package(Some(workspace_name), name)
            && let Some(pkg) =
                self.process_package_entry(entry, name, override_version, resolved_version)?
            && self.version_satisfies_spec(&pkg.version, version_spec)
        {
            return Ok(Some(pkg));
        }

        // Search for nested/aliased versions that match
        // Only search explicitly nested entries (with '/' in key), not bundled deps
        for (lockfile_key, entry) in &self.data.packages {
            // Only consider explicitly nested entries (not bundled)
            if !lockfile_key.contains('/') {
                continue;
            }

            // Skip bundled dependencies
            if let Some(info) = &entry.info
                && info
                    .other
                    .get("bundled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                continue;
            }

            let ident = PackageIdent::parse(&entry.ident);

            // Skip if the name doesn't match
            if ident.name() != name {
                continue;
            }

            // Skip workspace mappings
            if ident.is_workspace() {
                continue;
            }

            // Check if this version satisfies the spec
            if let Some(pkg) =
                self.process_package_entry(entry, name, override_version, resolved_version)?
                && self.version_satisfies_spec(&pkg.version, version_spec)
            {
                tracing::debug!(
                    "Found matching version {} for {} (spec: {}) in nested entry {}",
                    pkg.version,
                    name,
                    version_spec,
                    lockfile_key
                );
                return Ok(Some(pkg));
            }
        }

        Ok(None)
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

        // Parse version spec using structured type
        let version_spec = VersionSpec::parse(version);

        // Handle catalog references
        let resolved_version = if version_spec.is_catalog() {
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
        if self.data.lockfile_version >= 1 {
            let override_spec = VersionSpec::parse(override_version);
            if let Some(workspace_target_path) = override_spec.workspace_path()
                && let Some(target_workspace) = self.data.workspaces.get(workspace_target_path)
            {
                // This is a workspace dependency, create a synthetic package entry
                let workspace_version = target_workspace.version.as_deref().unwrap_or("0.0.0");
                return Ok(Some(crate::Package {
                    key: format!("{name}@{workspace_version}"),
                    version: workspace_version.to_string(),
                }));
            }
        }

        // Find a package version that satisfies the version spec
        // This searches workspace-scoped, hoisted, and nested entries
        if let Some(pkg) = self.find_matching_version(
            workspace_name,
            name,
            version,
            override_version,
            resolved_version,
        )? {
            return Ok(Some(pkg));
        }

        Ok(None)
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
                // Optional peers without nested entries should be skipped (prevents pulling
                // unrelated packages like "next" into @vercel/analytics). But declared
                // optionalDependencies (platform-specific binaries) should include hoisted
                // versions when no nested version exists.
                let parent_key = format!("{entry_key}/{dependency}");
                let has_nested = self.data.packages.contains_key(&parent_key);

                if !has_nested {
                    let is_optional_peer_only =
                        !info.optional_dependencies.contains_key(dependency);
                    let has_hoisted = self.data.packages.contains_key(dependency);

                    if is_optional_peer_only || !has_hoisted {
                        continue;
                    }
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
        // Workspace mappings must be included in the packages list to ensure they're
        // found during pruning
        let mut packages_with_workspaces: std::collections::HashSet<String> =
            packages.iter().cloned().collect();
        for ws_path in workspace_packages {
            if ws_path.is_empty() {
                continue;
            }
            if let Some(workspace_entry) = self.data.workspaces.get(ws_path.as_str()) {
                packages_with_workspaces.insert(workspace_entry.name.clone());
            }
        }

        // Add workspace peer dependencies that are actually installed
        // Peer dependencies declared at workspace level are requirements, not automatic
        // dependencies, but if they're installed (exist in packages section), they
        // should be included in the pruned lockfile
        for ws_path in workspace_packages {
            if let Some(workspace_entry) = self.data.workspaces.get(ws_path.as_str())
                && let Some(peer_deps) = &workspace_entry.peer_dependencies
            {
                for peer_name in peer_deps.keys() {
                    // Check if this peer dependency exists as an installed package
                    if self.data.packages.contains_key(peer_name) {
                        packages_with_workspaces.insert(peer_name.clone());
                    }
                }
            }
        }

        // Also check root workspace peer dependencies
        if let Some(root_workspace) = self.data.workspaces.get("")
            && let Some(peer_deps) = &root_workspace.peer_dependencies
        {
            for peer_name in peer_deps.keys() {
                if self.data.packages.contains_key(peer_name) {
                    packages_with_workspaces.insert(peer_name.clone());
                }
            }
        }

        let packages_vec: Vec<String> = packages_with_workspaces.into_iter().collect();

        let subgraph = self.subgraph(workspace_packages, &packages_vec)?;
        Ok(Box::new(subgraph))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        let mut output = String::new();
        self.write_header(&mut output);
        self.write_workspaces(&mut output)?;
        self.write_trusted_dependencies(&mut output)?;
        self.write_overrides(&mut output)?;
        self.write_catalogs(&mut output)?;
        self.write_packages(&mut output)?;
        self.write_patched_dependencies(&mut output)?;
        output.push_str("}\n");
        Ok(output.into_bytes())
    }

    fn patches(&self) -> Result<Vec<RelativeUnixPathBuf>, crate::Error> {
        let mut patches = self
            .data
            .patched_dependencies
            .values()
            .map(RelativeUnixPathBuf::new)
            .collect::<Result<Vec<_>, turbopath::PathError>>()?;
        patches.sort();
        Ok(patches)
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        let Some(other_bun) = any_other.downcast_ref::<Self>() else {
            return true;
        };

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

    fn write_header(&self, output: &mut String) {
        output.push_str("{\n");
        output.push_str(&format!(
            "  \"lockfileVersion\": {},\n",
            self.data.lockfile_version
        ));
        // Write configVersion if present
        if let Some(config_version) = self.data.config_version {
            output.push_str(&format!("  \"configVersion\": {},\n", config_version));
        }
    }

    fn write_workspaces(&self, output: &mut String) -> Result<(), crate::Error> {
        // serde_json uses 2-space indentation, but Bun uses 4-space
        output.push_str("  \"workspaces\": ");
        let workspaces_json = serde_json::to_string_pretty(&self.data.workspaces)?;

        let lines: Vec<&str> = workspaces_json.lines().collect();
        let mut adjusted_json = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                adjusted_json.push_str(line);
            } else {
                let spaces = line.len() - line.trim_start().len();
                let indent = " ".repeat(spaces + 2);
                adjusted_json.push_str(&format!("\n{}{}", indent, line.trim_start()));
            }
        }

        // Use the helper function to add trailing commas
        let workspaces_with_commas = Self::add_trailing_commas(&adjusted_json);

        output.push_str(&workspaces_with_commas);
        output.push_str(",\n");

        Ok(())
    }

    fn write_trusted_dependencies(&self, output: &mut String) -> Result<(), crate::Error> {
        if self.data.trusted_dependencies.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.trusted_dependencies)?;
        // Format with proper indentation
        let lines: Vec<&str> = json.lines().collect();
        output.push_str("  \"trustedDependencies\": ");
        let mut adjusted = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                adjusted.push_str(line);
            } else {
                let spaces = line.len() - line.trim_start().len();
                let indent = " ".repeat(spaces + 2);
                adjusted.push_str(&format!("\n{}{}", indent, line.trim_start()));
            }
        }
        // Add trailing commas after all strings (bun requires trailing commas)
        let with_commas = Self::add_trailing_commas(&adjusted);
        output.push_str(&with_commas);
        output.push_str(",\n");
        Ok(())
    }

    /// Add trailing commas to JSON values before closing brackets/braces
    /// Handles strings, numbers, booleans, nulls, and nested structures
    fn add_trailing_commas(json: &str) -> String {
        use regex::Regex;
        // Match: any JSON value (string, number, boolean, null, ] or }) followed by
        // newline+whitespace and then a closing bracket/brace
        // Pattern covers:
        // - Strings ending with "
        // - Numbers ending with digits
        // - Booleans: true, false
        // - Null: null
        // - Nested closings: ] or }
        let re = Regex::new(r#"("|true|false|null|\d|[\]}])\n(\s*)([\]}])"#).unwrap();
        // Run multiple passes until no more changes (handles deeply nested structures)
        let mut result = json.to_string();
        loop {
            let new_result = re.replace_all(&result, "$1,\n$2$3").to_string();
            if new_result == result {
                break;
            }
            result = new_result;
        }
        result
    }

    fn write_overrides(&self, output: &mut String) -> Result<(), crate::Error> {
        if self.data.overrides.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.overrides)?;
        let lines: Vec<&str> = json.lines().collect();
        output.push_str("  \"overrides\": ");
        let mut adjusted = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                adjusted.push_str(line);
            } else {
                let spaces = line.len() - line.trim_start().len();
                let indent = " ".repeat(spaces + 2);
                adjusted.push_str(&format!("\n{}{}", indent, line.trim_start()));
            }
        }
        let with_commas = Self::add_trailing_commas(&adjusted);
        output.push_str(&with_commas);
        output.push_str(",\n");
        Ok(())
    }

    fn write_catalogs(&self, output: &mut String) -> Result<(), crate::Error> {
        // Write default catalog if present
        if !self.data.catalog.is_empty() {
            let json = serde_json::to_string_pretty(&self.data.catalog)?;
            let lines: Vec<&str> = json.lines().collect();
            output.push_str("  \"catalog\": ");
            let mut adjusted = String::new();
            for (i, line) in lines.iter().enumerate() {
                if i == 0 {
                    adjusted.push_str(line);
                } else {
                    let spaces = line.len() - line.trim_start().len();
                    let indent = " ".repeat(spaces + 2);
                    adjusted.push_str(&format!("\n{}{}", indent, line.trim_start()));
                }
            }
            let with_commas = Self::add_trailing_commas(&adjusted);
            output.push_str(&with_commas);
            output.push_str(",\n");
        }

        // Write named catalogs if present
        if self.data.catalogs.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.catalogs)?;
        let lines: Vec<&str> = json.lines().collect();
        output.push_str("  \"catalogs\": ");
        let mut adjusted = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                adjusted.push_str(line);
            } else {
                let spaces = line.len() - line.trim_start().len();
                let indent = " ".repeat(spaces + 2);
                adjusted.push_str(&format!("\n{}{}", indent, line.trim_start()));
            }
        }
        let with_commas = Self::add_trailing_commas(&adjusted);
        output.push_str(&with_commas);
        output.push_str(",\n");
        Ok(())
    }

    fn write_packages(&self, output: &mut String) -> Result<(), crate::Error> {
        output.push_str("  \"packages\": {\n");

        let package_keys = self.sort_package_keys();
        for (i, key) in package_keys.iter().enumerate() {
            let entry = &self.data.packages[*key];

            let ident = PackageIdent::parse(&entry.ident);
            if ident.is_workspace() {
                let ident_json = serde_json::to_string(&entry.ident)?;
                output.push_str(&format!("    \"{key}\": [{ident_json}],"));
            } else {
                let ident_json = serde_json::to_string(&entry.ident)?;
                let info_json =
                    serde_json::to_string(&entry.info.as_ref().unwrap_or(&PackageInfo::default()))?;
                let checksum_json = serde_json::to_string(entry.checksum.as_deref().unwrap_or(""))?;

                // Bun's format differs from serde_json: objects need padding spaces,
                // 3-element arrays get expanded with trailing commas, others stay compact
                let info_json_spaced = self.format_info_json(&info_json);

                // GitHub and git packages have 3 elements (no registry)
                // npm packages have 4 elements (with registry)
                let is_git_package =
                    entry.ident.contains("@git+") || entry.ident.contains("@github:");

                if is_git_package {
                    // GitHub/git packages: [ident, info, checksum] - 3 elements
                    output.push_str(&format!(
                        "    \"{key}\": [{ident_json}, {info_json_spaced}, {checksum_json}],",
                    ));
                } else {
                    // npm packages: [ident, registry, info, checksum] - 4 elements
                    let registry_json =
                        serde_json::to_string(entry.registry.as_deref().unwrap_or(""))?;
                    output.push_str(&format!(
                        "    \"{key}\": [{ident_json}, {registry_json}, {info_json_spaced}, \
                         {checksum_json}],",
                    ));
                }
            }

            if i < package_keys.len() - 1 {
                output.push_str("\n\n");
            } else {
                output.push('\n');
            }
        }
        // Add comma if there are patched dependencies to follow
        if !self.data.patched_dependencies.is_empty() {
            output.push_str("  },\n");
        } else {
            output.push_str("  }\n");
        }

        Ok(())
    }

    fn write_patched_dependencies(&self, output: &mut String) -> Result<(), crate::Error> {
        if self.data.patched_dependencies.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.data.patched_dependencies)?;
        let lines: Vec<&str> = json.lines().collect();
        output.push_str("  \"patchedDependencies\": ");
        let mut adjusted = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                adjusted.push_str(line);
            } else {
                let spaces = line.len() - line.trim_start().len();
                let indent = " ".repeat(spaces + 2);
                adjusted.push_str(&format!("\n{}{}", indent, line.trim_start()));
            }
        }
        let with_commas = Self::add_trailing_commas(&adjusted);
        output.push_str(&with_commas);
        // No trailing comma - this is the last section before closing brace
        output.push('\n');
        Ok(())
    }

    /// Bun sorts packages by structure: regular packages, then scoped hoisted,
    /// then non-scoped hoisted, then deeply nested
    fn sort_package_keys(&self) -> Vec<&String> {
        // Sort priorities for package keys
        const SORT_PRIORITY_TOP_LEVEL: u8 = 1;
        const SORT_PRIORITY_SHALLOW_NESTED: u8 = 2;
        const SORT_PRIORITY_DEEP_NESTED: u8 = 3;
        const SORT_PRIORITY_VERY_DEEP_NESTED: u8 = 4;

        let mut package_keys: Vec<_> = self.data.packages.keys().collect();
        package_keys.sort_by(|a, b| {
            let category = |key_str: &str| -> u8 {
                let key = PackageKey::parse(key_str);
                match key {
                    PackageKey::Simple(_) => SORT_PRIORITY_TOP_LEVEL,
                    PackageKey::Scoped { .. } => SORT_PRIORITY_TOP_LEVEL,
                    PackageKey::Nested { .. } => SORT_PRIORITY_DEEP_NESTED,
                    PackageKey::ScopedNested { .. } => {
                        // Count slashes to determine nesting depth
                        let slash_count = key_str.matches('/').count();
                        if slash_count == 2 {
                            SORT_PRIORITY_SHALLOW_NESTED // @scope/parent/dep
                        } else {
                            SORT_PRIORITY_VERY_DEEP_NESTED // deeper nesting
                        }
                    }
                }
            };

            let a_cat = category(a);
            let b_cat = category(b);

            if a_cat != b_cat {
                a_cat.cmp(&b_cat)
            } else {
                a.cmp(b)
            }
        });
        package_keys
    }

    /// Formats JSON to match Bun's specific formatting requirements:
    /// - Objects need padding spaces: `{ "key": "value" }`
    /// - 3-element arrays get expanded with trailing commas: `[ item1, item2,
    ///   item3, ]`
    /// - Other arrays stay compact: `[item1, item2]`
    fn format_info_json(&self, info_json: &str) -> String {
        if info_json == "{}" {
            return info_json.to_string();
        }

        let mut result = String::with_capacity(info_json.len() + 100);
        let chars: Vec<char> = info_json.chars().collect();
        let mut i = 0;
        let mut in_string = false;
        let mut escape_next = false;

        while i < chars.len() {
            let c = chars[i];

            if !escape_next {
                if c == '"' {
                    in_string = !in_string;
                } else if c == '\\' && in_string {
                    escape_next = true;
                }
            } else {
                escape_next = false;
            }

            if !in_string {
                match c {
                    '{' => {
                        result.push_str("{ ");
                        i += 1;
                        continue;
                    }
                    '}' => {
                        result.push_str(" }");
                        i += 1;
                        continue;
                    }
                    ':' => {
                        result.push_str(": ");
                        i += 1;
                        continue;
                    }
                    '[' => {
                        let array_result = self.format_array(&chars, &mut i);
                        result.push_str(&array_result);
                        i += 1; // skip closing ]
                        continue;
                    }
                    ',' => {
                        result.push_str(", ");
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
            }

            result.push(c);
            i += 1;
        }

        result
    }

    /// Formats arrays according to Bun's requirements.
    /// Returns the formatted array string and updates the index.
    fn format_array(&self, chars: &[char], i: &mut usize) -> String {
        let mut array_depth = 1;
        let mut array_content = String::new();
        let mut in_array_string = false;
        let mut array_escape_next = false;
        *i += 1;

        while *i < chars.len() && array_depth > 0 {
            let array_char = chars[*i];

            if !array_escape_next {
                if array_char == '"' {
                    in_array_string = !in_array_string;
                } else if array_char == '\\' && in_array_string {
                    array_escape_next = true;
                } else if !in_array_string {
                    if array_char == '[' {
                        array_depth += 1;
                    } else if array_char == ']' {
                        array_depth -= 1;
                        if array_depth == 0 {
                            break;
                        }
                    }
                }
            } else {
                array_escape_next = false;
            }

            array_content.push(array_char);
            *i += 1;
        }

        let trimmed_content = array_content.trim_matches(|c: char| c == ',' || c.is_whitespace());

        // Bun uses compact arrays without trailing commas inside package entries
        format!("[{}]", self.format_array_content(trimmed_content))
    }

    /// Formats the content inside an array by adding proper spacing after
    /// commas.
    fn format_array_content(&self, content: &str) -> String {
        let mut formatted = String::with_capacity(content.len() + 20);
        let mut depth = 0;
        let mut in_str = false;
        let mut esc = false;

        for ch in content.chars() {
            if !esc {
                if ch == '"' {
                    in_str = !in_str;
                } else if ch == '\\' && in_str {
                    esc = true;
                } else if !in_str {
                    if ch == '[' || ch == '{' {
                        depth += 1;
                    } else if ch == ']' || ch == '}' {
                        depth -= 1;
                    } else if ch == ',' && depth == 0 {
                        formatted.push_str(", ");
                        continue;
                    }
                }
            } else {
                esc = false;
            }
            formatted.push(ch);
        }

        formatted
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
    #[cfg(test)]
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
        use std::collections::HashSet;

        // Create pruned lockfile structure
        let mut pruned_data = BunLockfileData {
            lockfile_version: self.data.lockfile_version,
            config_version: self.data.config_version,
            workspaces: Map::new(),
            trusted_dependencies: self.data.trusted_dependencies.clone(),
            overrides: Map::new(),
            catalog: self.data.catalog.clone(),
            catalogs: self.data.catalogs.clone(),
            packages: Map::new(),
            patched_dependencies: Map::new(),
        };

        if let Some(root) = self.data.workspaces.get("") {
            pruned_data.workspaces.insert("".to_string(), root.clone());
        }

        for ws_path in workspace_packages {
            if let Some(entry) = self.data.workspaces.get(ws_path) {
                pruned_data
                    .workspaces
                    .insert(ws_path.clone(), entry.clone());
            }
        }

        let mut keys_to_include = HashSet::new();

        let target_workspace_names: HashSet<String> = workspace_packages
            .iter()
            .filter_map(|ws_path| self.data.workspaces.get(ws_path).map(|ws| ws.name.clone()))
            .collect();

        // When idents map to multiple lockfile keys, only include workspace-specific
        // entries for target workspaces to avoid pulling in unrelated workspace
        // versions
        for pkg in packages {
            if self.data.packages.contains_key(pkg)
                || pruned_data.workspaces.values().any(|ws| &ws.name == pkg)
            {
                keys_to_include.insert(pkg.clone());
            } else if let Some(at_pos) = pkg.rfind('@') {
                let name = &pkg[..at_pos];

                if let Some(entry) = self.data.packages.get(name)
                    && entry.ident.contains("@workspace:")
                {
                    keys_to_include.insert(name.to_string());
                    // Continue to also find package entries with this ident
                    // (e.g., both "storybook" workspace mapping and
                    // "storybook/storybook")
                }

                for (lockfile_key, entry) in &self.data.packages {
                    if &entry.ident != pkg {
                        continue;
                    }

                    if let Some(slash_pos) = lockfile_key.find('/') {
                        let prefix = &lockfile_key[..slash_pos];

                        let is_workspace_prefix =
                            self.data.workspaces.values().any(|ws| ws.name == prefix);

                        if is_workspace_prefix {
                            if target_workspace_names.contains(prefix) {
                                keys_to_include.insert(lockfile_key.clone());
                            }
                        } else {
                            keys_to_include.insert(lockfile_key.clone());
                        }
                    } else {
                        keys_to_include.insert(lockfile_key.clone());
                    }
                }
            } else {
                for (lockfile_key, entry) in &self.data.packages {
                    if &entry.ident != pkg {
                        continue;
                    }

                    if let Some(slash_pos) = lockfile_key.find('/') {
                        let prefix = &lockfile_key[..slash_pos];

                        let is_workspace_prefix =
                            self.data.workspaces.values().any(|ws| ws.name == prefix);

                        if is_workspace_prefix {
                            if target_workspace_names.contains(prefix) {
                                keys_to_include.insert(lockfile_key.clone());
                            }
                        } else {
                            keys_to_include.insert(lockfile_key.clone());
                        }
                    } else {
                        keys_to_include.insert(lockfile_key.clone());
                    }
                }
            }
        }

        // De-alias workspace-specific keys (e.g., "blog/@types/react" ->
        // "@types/react") so peer dependencies resolve correctly in pruned
        // lockfiles
        let should_dealias = !workspace_packages.is_empty();

        let mut dealias_set: std::collections::HashSet<String> = std::collections::HashSet::new();
        if should_dealias {
            for key in &keys_to_include {
                let parsed_key = PackageKey::parse(key);

                // Only nested keys can be dealiased
                if let Some(parent) = parsed_key.parent() {
                    // Check if this is nested under a target workspace
                    if target_workspace_names.contains(&parent) {
                        // Get the dealiased version
                        if let Some(dealiased_key) = parsed_key.dealias() {
                            let dealiased_str = dealiased_key.to_string();

                            // Check if dealiasing would conflict with an existing workspace mapping
                            let would_conflict = if let Some(existing_entry) =
                                self.data.packages.get(&dealiased_str)
                            {
                                let ident = PackageIdent::parse(&existing_entry.ident);
                                ident.is_workspace()
                            } else {
                                false
                            };

                            if !would_conflict {
                                dealias_set.insert(dealiased_str);
                            }
                        }
                    }
                }
            }
        }

        let mut sorted_keys: Vec<_> = keys_to_include.iter().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            if let Some(entry) = self.data.packages.get(key) {
                let pruned_key = if should_dealias {
                    let parsed_key = PackageKey::parse(key);

                    // Check if this is a nested key that could be dealiased
                    if let Some(parent) = parsed_key.parent() {
                        let is_target_workspace_prefix = target_workspace_names.contains(&parent);

                        if is_target_workspace_prefix {
                            // Try to dealias
                            if let Some(dealiased_key) = parsed_key.dealias() {
                                let dealiased_str = dealiased_key.to_string();

                                // Check if dealiasing would conflict with an existing workspace
                                // mapping
                                if let Some(existing_entry) = self.data.packages.get(&dealiased_str)
                                {
                                    let ident = PackageIdent::parse(&existing_entry.ident);
                                    if ident.is_workspace() {
                                        // This would conflict with a workspace mapping - keep full
                                        // key
                                        key.clone()
                                    } else {
                                        // No conflict - safe to dealias
                                        dealiased_str
                                    }
                                } else {
                                    // No existing entry - safe to dealias
                                    dealiased_str
                                }
                            } else {
                                // Cannot dealias
                                key.clone()
                            }
                        } else {
                            // Keep the key as-is (it's nested under a package, not a workspace)
                            key.clone()
                        }
                    } else {
                        // No slash - this is a top-level entry
                        // Check if a workspace-scoped version will be de-aliased to this same key
                        if dealias_set.contains(key) {
                            // Skip this top-level entry - it conflicts with a workspace-scoped
                            // version
                            continue;
                        }
                        key.clone()
                    }
                } else {
                    // Not dealiasing - keep key as-is
                    key.clone()
                };

                // Check if this is a workspace mapping entry (e.g., "storybook":
                // ["storybook@workspace:apps/storybook"])
                let ident = PackageIdent::parse(&entry.ident);
                let is_workspace_mapping = ident.is_workspace() && ident.name() == key;

                // Handle workspace mapping entries
                if is_workspace_mapping {
                    // Extract the workspace path from the mapping
                    // Format: "storybook@workspace:apps/storybook" -> workspace path is
                    // "apps/storybook"
                    if let Some(workspace_path) = ident.workspace_path() {
                        // Check if this workspace is in the pruned set
                        if pruned_data.workspaces.contains_key(workspace_path) {
                            // This workspace IS in the pruned set - keep the mapping as-is
                            pruned_data
                                .packages
                                .insert(pruned_key.clone(), entry.clone());
                            continue;
                        }

                        // This workspace is NOT in the pruned set
                        // Try to find the actual npm package entry instead
                        // Get the workspace name (last component of path)
                        let workspace_name = workspace_path
                            .split('/')
                            .next_back()
                            .unwrap_or(workspace_path);

                        // Look for the actual package entry stored with workspace-scoped key
                        // e.g., "storybook/storybook" for workspace "storybook"
                        let scoped_key = format!("{workspace_name}/{key}");

                        if let Some(actual_package) = self.data.packages.get(&scoped_key) {
                            // Include the actual package entry with the unscoped key
                            pruned_data
                                .packages
                                .insert(pruned_key.clone(), actual_package.clone());
                        }
                    }

                    // Skip the workspace mapping entry itself
                    continue;
                }

                pruned_data
                    .packages
                    .insert(pruned_key.clone(), entry.clone());

                // Check if this package references a workspace (e.g., via @workspace: in ident)
                // and ensure that workspace is included
                let package_ident = PackageIdent::parse(&entry.ident);
                if let Some(workspace_path) = package_ident.workspace_path() {
                    // Add this workspace if not already included
                    if !pruned_data.workspaces.contains_key(workspace_path)
                        && let Some(ws_entry) = self.data.workspaces.get(workspace_path)
                    {
                        pruned_data
                            .workspaces
                            .insert(workspace_path.to_string(), ws_entry.clone());
                    }
                }

                // Include bundled dependencies
                // Bundled dependencies are stored with nested keys like "parent/dep"
                // and have "bundled": true in their info
                // Note: We search using the original key from the source lockfile
                let bundled_prefix = format!("{key}/");
                for (lockfile_key, bundled_entry) in &self.data.packages {
                    if lockfile_key.starts_with(&bundled_prefix)
                        && let Some(bundled_info) = &bundled_entry.info
                        && bundled_info
                            .other
                            .get("bundled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    {
                        // Check if this is a bundled dependency
                        // In Bun's format, bundled is indicated by the "bundled" field
                        // Bundled deps are always nested under their parent,
                        // so we need to adjust the key if we dealiased the parent
                        let bundled_pruned_key =
                            if should_dealias && lockfile_key.starts_with(&bundled_prefix) {
                                // Replace the parent prefix with the dealiased version
                                format!("{}{}", pruned_key, &lockfile_key[key.len()..])
                            } else {
                                lockfile_key.clone()
                            };
                        pruned_data
                            .packages
                            .insert(bundled_pruned_key, bundled_entry.clone());
                    }
                }
            } else {
                // Key doesn't exist in original lockfile - check if it's a workspace name
                // and create a workspace mapping entry for it
                if let Some((ws_path, _ws_entry)) = pruned_data
                    .workspaces
                    .iter()
                    .find(|(_, ws)| &ws.name == key)
                {
                    // Skip root workspace
                    if !ws_path.is_empty() {
                        let ident = format!("{key}@workspace:{ws_path}");
                        let entry = PackageEntry {
                            ident,
                            registry: None,
                            info: None,
                            checksum: None,
                            root: None,
                        };
                        pruned_data.packages.insert(key.clone(), entry);
                    }
                }
            }
        }

        // Collect idents of all packages in the subgraph for filtering purposes
        let included_idents: HashSet<&str> = pruned_data
            .packages
            .values()
            .map(|entry| entry.ident.as_str())
            .collect();

        // Filter overrides to only include those for packages in the subgraph
        // Extract package names from idents for comparison with override keys
        let included_package_names: HashSet<String> = pruned_data
            .packages
            .values()
            .filter_map(|entry| {
                // Extract package name from ident (format: "name@version")
                entry.ident.split('@').next().map(|s| s.to_string())
            })
            .collect();

        for (pkg_name, override_version) in &self.data.overrides {
            if included_package_names.contains(pkg_name) {
                pruned_data
                    .overrides
                    .insert(pkg_name.clone(), override_version.clone());
            }
        }

        // Filter patched_dependencies to only include those for packages in the
        // subgraph
        for (pkg_ident, patch_path) in &self.data.patched_dependencies {
            if included_idents.contains(pkg_ident.as_str()) {
                pruned_data
                    .patched_dependencies
                    .insert(pkg_ident.clone(), patch_path.clone());
            }
        }

        // ORPHAN REMOVAL: After de-aliasing, some packages may be orphaned
        // (only depended on by packages that were skipped due to de-aliasing
        // conflicts). We need to remove these orphaned packages.
        //
        // Strategy: Recompute which packages are reachable from workspace dependencies
        // using the pruned packages. Packages not reachable are orphans.

        // Build key_to_entry for closure computation
        let mut temp_key_to_entry: HashMap<String, String> = HashMap::new();
        for (path, entry) in &pruned_data.packages {
            // Take first occurrence for duplicate idents (shouldn't happen after
            // de-aliasing)
            temp_key_to_entry
                .entry(entry.ident.clone())
                .or_insert(path.clone());
        }

        // Create temporary lockfile for recomputation
        let temp_data = BunLockfileData {
            lockfile_version: pruned_data.lockfile_version,
            config_version: pruned_data.config_version,
            workspaces: pruned_data.workspaces.clone(),
            trusted_dependencies: pruned_data.trusted_dependencies.clone(),
            overrides: pruned_data.overrides.clone(),
            catalog: self.data.catalog.clone(),
            catalogs: self.data.catalogs.clone(),
            packages: pruned_data.packages.clone(),
            patched_dependencies: pruned_data.patched_dependencies.clone(),
        };
        let temp_index = PackageIndex::new(&temp_data.packages);
        let temp_lockfile = BunLockfile {
            data: temp_data,
            key_to_entry: temp_key_to_entry,
            index: temp_index,
        };

        // Collect workspace dependencies
        let workspace_deps: HashMap<String, HashMap<String, String>> = pruned_data
            .workspaces
            .iter()
            .map(|(ws_path, ws_entry)| {
                let mut deps = HashMap::new();
                if let Some(d) = &ws_entry.dependencies {
                    deps.extend(d.clone());
                }
                if let Some(dd) = &ws_entry.dev_dependencies {
                    deps.extend(dd.clone());
                }
                if let Some(od) = &ws_entry.optional_dependencies {
                    deps.extend(od.clone());
                }
                // Include peer dependencies for orphan removal computation
                // Peer dependencies that are actually installed should not be considered
                // orphans
                if let Some(pd) = &ws_entry.peer_dependencies {
                    deps.extend(pd.clone());
                }
                (ws_path.clone(), deps)
            })
            .collect();

        // Recompute transitive closure
        match crate::all_transitive_closures(&temp_lockfile, workspace_deps, true) {
            Ok(recomputed_closures) => {
                let reachable_idents: HashSet<String> = recomputed_closures
                    .values()
                    .flat_map(|closure| closure.iter().map(|p| p.key.clone()))
                    .collect();

                // Also keep track of reachable lockfile keys for nested package detection
                let reachable_lockfile_keys: HashSet<String> = recomputed_closures
                    .values()
                    .flat_map(|closure| {
                        closure
                            .iter()
                            .filter_map(|p| temp_lockfile.key_to_entry.get(&p.key).cloned())
                    })
                    .collect();

                // Remove unreachable packages
                pruned_data.packages.retain(|key, entry| {
                    // Keep if the ident is reachable
                    if reachable_idents.contains(&entry.ident)
                        || entry.ident.contains("@workspace:")
                    {
                        return true;
                    }

                    // Keep nested packages if their parent is reachable
                    // E.g., keep "@hatchet-dev/typescript-sdk/zod" if "@hatchet-dev/typescript-sdk"
                    // is reachable
                    if let Some(slash_pos) = key.rfind('/') {
                        let parent_key = &key[..slash_pos];
                        if reachable_lockfile_keys.contains(parent_key) {
                            return true;
                        }
                    }

                    false
                });
            }
            Err(_e) => {}
        }

        // Rebuild key_to_entry HashMap for the pruned lockfile
        let mut key_to_entry: HashMap<String, String> =
            HashMap::with_capacity(pruned_data.packages.len());
        for (path, entry) in pruned_data.packages.iter() {
            if let Some(prev_path) = key_to_entry.insert(entry.ident.clone(), path.clone()) {
                let prev_entry = pruned_data
                    .packages
                    .get(&prev_path)
                    .expect("we just got this path from the packages list");

                // Verify checksums match for duplicate idents
                if prev_entry.checksum != entry.checksum {
                    return Err(Error::MismatchedShas {
                        ident: entry.ident.clone(),
                        sha1: prev_entry.checksum.clone().unwrap_or_default(),
                        sha2: entry.checksum.clone().unwrap_or_default(),
                    });
                }
            }
        }

        // Build package index for pruned data
        let index = PackageIndex::new(&pruned_data.packages);

        Ok(BunLockfile {
            data: pruned_data,
            key_to_entry,
            index,
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

        // Build key_to_entry map
        // When there are multiple lockfile keys with the same ident (e.g., nested
        // versions), we pick the FIRST one in sorted order for determinism.
        // Sort keys to ensure deterministic selection: workspace-specific entries (with
        // /) come before hoisted entries (without /) in the sort order.
        let mut sorted_keys: Vec<_> = data.packages.keys().collect();
        sorted_keys.sort();

        let mut key_to_entry: HashMap<String, String> = HashMap::with_capacity(data.packages.len());
        for path in sorted_keys {
            let info = data.packages.get(path).unwrap();

            if let Some(prev_path) = key_to_entry.get(&info.ident) {
                let prev_info = data
                    .packages
                    .get(prev_path)
                    .expect("we just got this path from the packages list");

                // Verify checksums match for duplicate idents
                if prev_info.checksum != info.checksum {
                    return Err(Error::MismatchedShas {
                        ident: info.ident.clone(),
                        sha1: prev_info.checksum.clone().unwrap_or_default(),
                        sha2: info.checksum.clone().unwrap_or_default(),
                    }
                    .into());
                }
                // Skip this entry - we already have one for this ident
            } else {
                // First time seeing this ident
                key_to_entry.insert(info.ident.clone(), path.clone());
            }
        }
        // Build package index
        let index = PackageIndex::new(&data.packages);

        Ok(Self {
            data,
            key_to_entry,
            index,
        })
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

    Ok(prev.global_change(&curr as &dyn Lockfile))
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
    const PRUNE_BASIC_ORIGINAL: &str = include_str!("./snapshots/original-basic.lock");
    const PRUNE_TAILWIND_ORIGINAL: &str = include_str!("./snapshots/original-with-tailwind.lock");
    const PRUNE_KITCHEN_SINK_ORIGINAL: &str =
        include_str!("./snapshots/original-kitchen-sink.lock");
    const PRUNE_ISSUE_11007_ORIGINAL_1: &str =
        include_str!("./snapshots/original-issue-11007-1.lock");
    const PRUNE_ISSUE_11007_ORIGINAL_2: &str =
        include_str!("./snapshots/original-issue-11007-2.lock");
    const PRUNE_ISSUE_11074_ORIGINAL: &str = include_str!("./snapshots/original-issue-11074.lock");

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
            vec![("is-odd", "is-odd@3.0.0")]
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
    fn test_resolve_bundled_dependency() {
        // Test that bundled dependencies can be resolved even when called with
        // workspace context (not parent package context)
        let lockfile = BunLockfile::from_str(V1_ISSUE_10410_LOCKFILE).unwrap();

        // @emnapi/core exists only as a bundled dependency under
        // @tailwindcss/oxide-wasm32-wasi, not as a standalone package
        // When resolve_package is called with the workspace path, it should
        // still find the bundled entry
        let result = lockfile
            .resolve_package("apps/web", "@emnapi/core", "^1.4.5")
            .unwrap();

        assert!(
            result.is_some(),
            "Should be able to resolve bundled dependency @emnapi/core"
        );

        let package = result.unwrap();
        assert_eq!(package.key, "@emnapi/core@1.5.0");

        // Verify this works for other bundled dependencies too
        let runtime_result = lockfile
            .resolve_package("apps/web", "@emnapi/runtime", "^1.4.5")
            .unwrap();

        assert!(
            runtime_result.is_some(),
            "Should be able to resolve bundled dependency @emnapi/runtime"
        );

        let runtime_package = runtime_result.unwrap();
        assert_eq!(runtime_package.key, "@emnapi/runtime@1.5.0");
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

    /// Test helper to prune a lockfile for a single target workspace.
    /// Uses the same approach as production code: compute transitive closure
    /// via all_transitive_closures, then call subgraph with the results.
    fn prune_for_workspace(lockfile: &BunLockfile, target_workspace: &str) -> BunLockfile {
        use crate::all_transitive_closures;

        // Helper to extract all dependencies from a workspace entry
        fn get_workspace_deps(
            lockfile: &BunLockfile,
            ws_path: &str,
        ) -> std::collections::HashMap<String, String> {
            lockfile
                .data
                .workspaces
                .get(ws_path)
                .map(|entry| {
                    let mut deps = std::collections::HashMap::new();
                    if let Some(d) = &entry.dependencies {
                        deps.extend(d.clone());
                    }
                    if let Some(dd) = &entry.dev_dependencies {
                        deps.extend(dd.clone());
                    }
                    if let Some(od) = &entry.optional_dependencies {
                        deps.extend(od.clone());
                    }
                    deps
                })
                .unwrap_or_default()
        }

        // Start with root and target workspace
        let mut workspace_deps = std::collections::HashMap::new();
        workspace_deps.insert("".to_string(), get_workspace_deps(lockfile, ""));
        workspace_deps.insert(
            target_workspace.to_string(),
            get_workspace_deps(lockfile, target_workspace),
        );

        // Discover transitive workspace dependencies (workspace:* protocol)
        loop {
            let mut added_new = false;
            let current_workspaces: Vec<_> = workspace_deps.keys().cloned().collect();

            for (ws_path, ws_entry) in &lockfile.data.workspaces {
                if current_workspaces.contains(ws_path) {
                    continue;
                }

                let workspace_name = &ws_entry.name;
                let is_referenced = workspace_deps.iter().any(|(_ws, deps)| {
                    deps.iter().any(|(dep_name, dep_value)| {
                        (dep_name == workspace_name
                            || dep_name.starts_with(&format!("{workspace_name}/")))
                            && dep_value.starts_with("workspace:")
                    })
                });

                if is_referenced {
                    workspace_deps.insert(ws_path.clone(), get_workspace_deps(lockfile, ws_path));
                    added_new = true;
                }
            }

            if !added_new {
                break;
            }
        }

        // Compute transitive external dependencies for all workspaces
        let mut closures = all_transitive_closures(lockfile, workspace_deps.clone(), false)
            .expect("Failed to compute transitive closures");

        // Discover additional workspaces referenced via @workspace: in package idents
        loop {
            let mut added_new = false;
            let current_workspaces: Vec<_> = closures.keys().cloned().collect();

            for ws_path in lockfile.data.workspaces.keys() {
                if current_workspaces.contains(ws_path) {
                    continue;
                }

                let workspace_marker = format!("@workspace:{ws_path}");

                // Check if any resolved package references this workspace
                let is_referenced = closures.values().any(|closure| {
                    closure.iter().any(|pkg| {
                        // Look up the actual lockfile entry to check its ident
                        lockfile
                            .key_to_entry
                            .get(&pkg.key)
                            .and_then(|lockfile_key| lockfile.data.packages.get(lockfile_key))
                            .map(|entry| entry.ident.contains(&workspace_marker))
                            .unwrap_or(false)
                    })
                });

                if is_referenced {
                    workspace_deps
                        .insert(ws_path.to_string(), get_workspace_deps(lockfile, ws_path));
                    closures = all_transitive_closures(lockfile, workspace_deps.clone(), false)
                        .expect("Failed to recompute transitive closures");
                    added_new = true;
                    break;
                }
            }

            if !added_new {
                break;
            }
        }

        // Collect all packages and workspace paths
        let mut packages: std::collections::HashSet<String> = closures
            .values()
            .flat_map(|closure| closure.iter().map(|p| p.key.clone()))
            .collect();
        let workspace_paths: Vec<String> = closures.keys().cloned().collect();

        // Add workspace names to packages list so subgraph creates mapping entries
        for ws_path in &workspace_paths {
            if !ws_path.is_empty()
                && let Some(workspace_entry) = lockfile.data.workspaces.get(ws_path.as_str())
            {
                packages.insert(workspace_entry.name.clone());
            }
        }

        // Add workspace peer dependencies that are actually installed
        // (mimics the logic in the Lockfile trait's subgraph method)
        for ws_path in &workspace_paths {
            if let Some(workspace_entry) = lockfile.data.workspaces.get(ws_path.as_str())
                && let Some(peer_deps) = &workspace_entry.peer_dependencies
            {
                for peer_name in peer_deps.keys() {
                    if lockfile.data.packages.contains_key(peer_name) {
                        packages.insert(peer_name.clone());
                    }
                }
            }
        }

        // Add nested package entries (e.g., "parent/dep") for any packages we've
        // collected This handles cases where a package has a nested version of
        // a dependency (e.g., @hatchet-dev/typescript-sdk/zod for a different
        // zod version)
        let collected_packages: Vec<String> = packages.iter().cloned().collect();
        for pkg_ident in &collected_packages {
            // Convert ident to lockfile key using key_to_entry
            if let Some(lockfile_key) = lockfile.key_to_entry.get(pkg_ident) {
                let prefix = format!("{}/", lockfile_key);
                for nested_key in lockfile.data.packages.keys() {
                    if nested_key.starts_with(&prefix) {
                        // Add the nested key directly (it's a lockfile key, not an ident)
                        // We need to add both the key itself and its ident
                        packages.insert(nested_key.clone());
                        if let Some(nested_entry) = lockfile.data.packages.get(nested_key) {
                            packages.insert(nested_entry.ident.clone());
                        }
                    }
                }
            }
        }

        let packages: Vec<String> = packages.into_iter().collect();

        // Call internal subgraph method
        lockfile
            .subgraph(&workspace_paths, &packages)
            .expect("Failed to create subgraph")
    }

    #[test]
    fn test_prune_basic_docs() {
        let lockfile = BunLockfile::from_str(PRUNE_BASIC_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/docs");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_basic_web() {
        let lockfile = BunLockfile::from_str(PRUNE_BASIC_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/web");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_tailwind_docs() {
        let lockfile = BunLockfile::from_str(PRUNE_TAILWIND_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/docs");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_tailwind_web() {
        let lockfile = BunLockfile::from_str(PRUNE_TAILWIND_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/web");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_kitchen_sink_admin() {
        let lockfile = BunLockfile::from_str(PRUNE_KITCHEN_SINK_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/admin");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_kitchen_sink_blog() {
        let lockfile = BunLockfile::from_str(PRUNE_KITCHEN_SINK_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/blog");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_kitchen_sink_api() {
        let lockfile = BunLockfile::from_str(PRUNE_KITCHEN_SINK_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/api");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_kitchen_sink_storefront() {
        let lockfile = BunLockfile::from_str(PRUNE_KITCHEN_SINK_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/storefront");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_issue_11007_1_app_a() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11007_ORIGINAL_1).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/app-a");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_issue_11007_1_app_b() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11007_ORIGINAL_1).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/app-b");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_issue_11007_1_app_c() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11007_ORIGINAL_1).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/app-c");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_issue_11007_1_app_d() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11007_ORIGINAL_1).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/app-d");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    /// Edge case: Workspace package name is the same as a name of one from the
    /// npm registry!
    #[test]
    fn test_prune_issue_11007_1_storybook() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11007_ORIGINAL_1).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/storybook");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_issue_11007_2_api() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11007_ORIGINAL_2).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/api");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_issue_11007_2_web() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11007_ORIGINAL_2).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/web");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    #[test]
    fn test_prune_issue_11074() {
        let lockfile = BunLockfile::from_str(PRUNE_ISSUE_11074_ORIGINAL).unwrap();
        let pruned = prune_for_workspace(&lockfile, "apps/app-a");
        let pruned_str = String::from_utf8(pruned.encode().unwrap()).unwrap();
        insta::assert_snapshot!(pruned_str);
    }

    /// Test that pruning a lockfile with GitHub dependencies doesn't corrupt the format.
    /// GitHub packages should have 3 elements: [ident, info, checksum]
    /// NOT 4 elements with an empty registry: [ident, "", info, checksum]
    #[test]
    fn test_prune_github_package_format() {
        let lockfile_json = json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "root",
                },
                "packages/a": {
                    "name": "pkg-a",
                    "dependencies": {
                        "some-lib": "github:user/repo#abc123",
                    },
                },
            },
            "packages": {
                "some-lib": [
                    "some-lib@github:user/repo#abc123",
                    { "dependencies": {} },
                    "abc123"
                ],
            },
        });

        let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
        let subgraph = lockfile
            .subgraph(
                &["packages/a".into()],
                &["some-lib@github:user/repo#abc123".into()],
            )
            .unwrap();

        let encoded = String::from_utf8(subgraph.encode().unwrap()).unwrap();

        // Verify the GitHub package has exactly 3 elements (no empty string registry)
        // The output should contain the ident followed directly by the info object
        assert!(
            !encoded.contains(r#"["some-lib@github:user/repo#abc123", "", {"#),
            "GitHub package should NOT have empty string registry field"
        );
        assert!(
            encoded.contains(r#""some-lib": ["some-lib@github:user/repo#abc123", {"#),
            "GitHub package should have ident followed directly by info object"
        );
    }

    /// Test that metadata sections are preserved through encode round-trip.
    /// Bun expects configVersion, trustedDependencies, overrides, and catalogs.
    #[test]
    fn test_encode_preserves_metadata_sections() {
        let lockfile_json = json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                },
            },
            "trustedDependencies": ["esbuild", "sharp"],
            "overrides": {
                "lodash": "4.17.21",
            },
            "catalog": {
                "react": "^18.0.0",
            },
            "packages": {},
        });

        let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
        let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

        // Verify configVersion is present
        assert!(
            encoded.contains(r#""configVersion": 1"#),
            "configVersion should be preserved in encoded output"
        );

        // Verify trustedDependencies section is present with its contents
        assert!(
            encoded.contains(r#""trustedDependencies""#),
            "trustedDependencies section should be present"
        );
        assert!(
            encoded.contains(r#""esbuild""#),
            "trustedDependencies should contain esbuild"
        );
        assert!(
            encoded.contains(r#""sharp""#),
            "trustedDependencies should contain sharp"
        );

        // Verify overrides section is present with its contents
        assert!(
            encoded.contains(r#""overrides""#),
            "overrides section should be present"
        );
        assert!(
            encoded.contains(r#""lodash""#),
            "overrides should contain lodash"
        );

        // Verify catalog section is present with its contents
        assert!(
            encoded.contains(r#""catalog""#),
            "catalog section should be present"
        );
        assert!(
            encoded.contains(r#""react""#),
            "catalog should contain react"
        );
    }

    /// Test that optionalPeers arrays use compact format without trailing commas.
    /// Bun expects: ["react", "vue"] NOT [ "react", "vue", ]
    #[test]
    fn test_optional_peers_compact_format() {
        let lockfile_json = json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                },
            },
            "packages": {
                "some-pkg": [
                    "some-pkg@1.0.0",
                    "",
                    {
                        "peerDependencies": {
                            "react": "^18.0.0",
                            "vue": "^3.0.0",
                        },
                        "optionalPeers": ["react", "vue"],
                    },
                    "sha512-abc"
                ],
            },
        });

        let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
        let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

        // Verify optionalPeers uses compact format without leading/trailing spaces or commas
        // The array should be formatted as ["react", "vue"] or ["vue", "react"]
        // NOT as [ "react", "vue", ] or similar
        assert!(
            !encoded.contains(r#"[ ""#),
            "optionalPeers array should NOT have leading space after opening bracket"
        );
        assert!(
            !encoded.contains(r#", ]"#),
            "optionalPeers array should NOT have trailing comma before closing bracket"
        );

        // Verify the optionalPeers field exists and has content
        assert!(
            encoded.contains(r#""optionalPeers""#),
            "optionalPeers field should be present"
        );
    }

    /// Test that named catalogs (catalogs field) are preserved through encode.
    /// This tests the plural "catalogs" field, not the singular "catalog" field.
    #[test]
    fn test_encode_preserves_named_catalogs() {
        let lockfile_json = json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                },
            },
            "catalog": {
                "lodash": "^4.17.0",
            },
            "catalogs": {
                "frontend": {
                    "react": "^18.0.0",
                    "vue": "^3.0.0",
                },
                "backend": {
                    "express": "^4.18.0",
                },
            },
            "packages": {},
        });

        let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
        let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

        // Verify default catalog is present
        assert!(
            encoded.contains(r#""catalog""#),
            "default catalog section should be present"
        );
        assert!(
            encoded.contains(r#""lodash""#),
            "default catalog should contain lodash"
        );

        // Verify named catalogs section is present
        assert!(
            encoded.contains(r#""catalogs""#),
            "named catalogs section should be present"
        );

        // Verify frontend catalog entries
        assert!(
            encoded.contains(r#""frontend""#),
            "frontend catalog should be present"
        );
        assert!(
            encoded.contains(r#""react""#),
            "frontend catalog should contain react"
        );
        assert!(
            encoded.contains(r#""vue""#),
            "frontend catalog should contain vue"
        );

        // Verify backend catalog entries
        assert!(
            encoded.contains(r#""backend""#),
            "backend catalog should be present"
        );
        assert!(
            encoded.contains(r#""express""#),
            "backend catalog should contain express"
        );
    }

    /// Test that patched_dependencies are preserved through encode.
    #[test]
    fn test_encode_preserves_patched_dependencies() {
        let lockfile_json = json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "lodash": "^4.17.21",
                    },
                },
            },
            "packages": {
                "lodash": [
                    "lodash@4.17.21",
                    "",
                    {},
                    "sha512-abc"
                ],
            },
            "patchedDependencies": {
                "lodash@4.17.21": "patches/lodash+4.17.21.patch",
            },
        });

        let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
        let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

        // Verify patchedDependencies section is present
        assert!(
            encoded.contains(r#""patchedDependencies""#),
            "patchedDependencies section should be present"
        );
        assert!(
            encoded.contains(r#""lodash@4.17.21""#),
            "patchedDependencies should contain lodash entry"
        );
        assert!(
            encoded.contains(r#"patches/lodash+4.17.21.patch"#),
            "patchedDependencies should contain patch path"
        );
    }

    /// Test that packages section is correctly encoded with proper format.
    /// This verifies the packages field ordering and structure.
    #[test]
    fn test_encode_packages_structure() {
        let lockfile_json = json!({
            "lockfileVersion": 1,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "is-odd": "^3.0.0",
                    },
                },
            },
            "packages": {
                "is-odd": [
                    "is-odd@3.0.1",
                    "",
                    {
                        "dependencies": {
                            "is-number": "^6.0.0",
                        },
                    },
                    "sha512-def"
                ],
                "is-number": [
                    "is-number@6.0.0",
                    "",
                    {},
                    "sha512-ghi"
                ],
            },
        });

        let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
        let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

        // Verify packages section exists
        assert!(
            encoded.contains(r#""packages""#),
            "packages section should be present"
        );

        // Verify package entries are present with correct identifiers
        assert!(
            encoded.contains(r#""is-odd": ["is-odd@3.0.1""#),
            "is-odd package should be present with correct format"
        );
        assert!(
            encoded.contains(r#""is-number": ["is-number@6.0.0""#),
            "is-number package should be present with correct format"
        );

        // Verify packages have registry field (empty string for npm packages)
        assert!(
            encoded.contains(r#"["is-odd@3.0.1", "","#),
            "npm packages should have empty string registry field"
        );
    }

    /// Comprehensive test that all metadata sections are written in correct order.
    /// Bun expects a specific ordering of top-level keys.
    #[test]
    fn test_encode_section_ordering() {
        let lockfile_json = json!({
            "lockfileVersion": 1,
            "configVersion": 1,
            "workspaces": {
                "": { "name": "test" },
            },
            "trustedDependencies": ["esbuild"],
            "overrides": { "lodash": "4.17.21" },
            "catalog": { "react": "^18.0.0" },
            "catalogs": {
                "frontend": { "vue": "^3.0.0" },
            },
            "packages": {
                "lodash": ["lodash@4.17.21", "", {}, "sha512-abc"],
            },
            "patchedDependencies": {
                "lodash@4.17.21": "patches/lodash.patch",
            },
        });

        let lockfile = BunLockfile::from_str(&lockfile_json.to_string()).unwrap();
        let encoded = String::from_utf8(lockfile.encode().unwrap()).unwrap();

        // Find positions of each section to verify ordering
        let lockfile_version_pos = encoded.find(r#""lockfileVersion""#).unwrap();
        let config_version_pos = encoded.find(r#""configVersion""#).unwrap();
        let workspaces_pos = encoded.find(r#""workspaces""#).unwrap();
        let trusted_deps_pos = encoded.find(r#""trustedDependencies""#).unwrap();
        let overrides_pos = encoded.find(r#""overrides""#).unwrap();
        let catalog_pos = encoded.find(r#""catalog""#).unwrap();
        let catalogs_pos = encoded.find(r#""catalogs""#).unwrap();
        let packages_pos = encoded.find(r#""packages""#).unwrap();
        let patched_deps_pos = encoded.find(r#""patchedDependencies""#).unwrap();

        // Verify ordering: lockfileVersion < configVersion < workspaces < trustedDependencies
        // < overrides < catalog < catalogs < packages < patchedDependencies
        assert!(
            lockfile_version_pos < config_version_pos,
            "lockfileVersion should come before configVersion"
        );
        assert!(
            config_version_pos < workspaces_pos,
            "configVersion should come before workspaces"
        );
        assert!(
            workspaces_pos < trusted_deps_pos,
            "workspaces should come before trustedDependencies"
        );
        assert!(
            trusted_deps_pos < overrides_pos,
            "trustedDependencies should come before overrides"
        );
        assert!(
            overrides_pos < catalog_pos,
            "overrides should come before catalog"
        );
        assert!(
            catalog_pos < catalogs_pos,
            "catalog should come before catalogs"
        );
        assert!(
            catalogs_pos < packages_pos,
            "catalogs should come before packages"
        );
        assert!(
            packages_pos < patched_deps_pos,
            "packages should come before patchedDependencies"
        );
    }
}
