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

mod data;
mod de;
mod emit;
mod global_change;
mod id;
mod index;
mod lockfile;
mod parse;
mod resolve;
mod ser;
mod subgraph;
#[cfg(test)]
mod test;
mod types;

pub use data::{BunLockfile, Error};
pub use global_change::bun_global_change;
use id::PossibleKeyIter;
use index::PackageIndex;
pub use types::{PackageIdent, PackageKey, VersionSpec};

type Map<K, V> = std::collections::BTreeMap<K, V>;
type BTreeSet<T> = std::collections::BTreeSet<T>;

#[cfg(test)]
pub(super) use data::WorkspaceEntry;
pub(super) use data::{BunLockfileData, LockfileVersion, PackageEntry, PackageInfo, RootInfo};

/// Check if a package identifier refers to a git or GitHub package.
///
/// Git and GitHub packages have different serialization formats than npm
/// packages:
/// - npm packages: `[ident, registry, info, checksum]` (4 elements)
/// - git/github packages: `[ident, info, checksum]` (3 elements, no registry)
///
/// This function is used in deserialization, serialization, and encoding to
/// ensure consistent handling of these package types.
pub(crate) fn is_git_or_github_package(ident: &str) -> bool {
    ident.contains("@git+") || ident.contains("@github:")
}

pub(crate) fn is_tarball_or_url_package(ident: &str) -> bool {
    ident.contains("@https://") || ident.contains("@http://")
}
