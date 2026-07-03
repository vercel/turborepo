//! Toolchains: the abstraction that makes Turborepo generic over language
//! ecosystems.
//!
//! A [`Toolchain`] answers ecosystem-specific questions about packages —
//! starting with "which packages exist?" — so that the package graph and the
//! rest of the system never branch on a specific ecosystem. JavaScript is the
//! first implementation ([`JavaScriptToolchain`]); additional toolchains
//! (e.g. Cargo) register alongside it in the [`ToolchainRegistry`].
//!
//! The trait grows one concern at a time (discovery today; command
//! resolution, derived task inputs/outputs, external-dependency hashing,
//! watch triggers, and prune participation as they are needed), and every
//! concern must ship with real implementations for every registered
//! toolchain.
//!
//! # Design rules
//!
//! These rules keep the door open to an out-of-process plugin architecture
//! (subprocess or WASM adapters implementing this same trait) without
//! committing to one today:
//!
//! 1. Trait methods are coarse-grained and data-in/data-out: arguments and
//!    return values are serializable-shaped (paths, strings, plain structs). No
//!    internal graph types, no lifetime-carrying views, no callbacks.
//! 2. [`ToolchainId`] is an open identifier, never a closed enum. A future
//!    toolchain (or plugin) mints a new id without touching existing code.
//! 3. All toolchain lookups go through the [`ToolchainRegistry`]. Scattered
//!    per-toolchain branch points (`if id == "cargo"`) are a design defect.
//!
//! # Known debt
//!
//! JavaScript machinery that predates this abstraction is still reachable
//! outside the trait where other build phases need it. Each access point is
//! the shrink list for future iterations of the trait surface:
//!
//! - [`JavaScriptToolchain::package_manager`]: package-manager resolution feeds
//!   dependency splitting and lockfile handling in the package graph builder.
//!   Lockfile handling gains a trait surface with external dependency hashing;
//!   dependency splitting remains JS-native for now.

use std::{borrow::Cow, fmt, future::Future, pin::Pin, sync::Arc};

use turbopath::AbsoluteSystemPathBuf;

use crate::{
    discovery::{self, PackageDiscovery},
    package_json::PackageJson,
    package_manager::PackageManager,
};

/// Identifies a toolchain: the language ecosystem a package belongs to.
///
/// Open by design (see the module's design rules): any string can be a
/// toolchain id, so new toolchains — including, potentially, ones loaded as
/// plugins — do not require changes to this type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToolchainId(Cow<'static, str>);

impl ToolchainId {
    /// The JavaScript toolchain: packages discovered from `package.json`
    /// manifests, regardless of package manager or runtime.
    pub const JAVASCRIPT: ToolchainId = ToolchainId(Cow::Borrowed("javascript"));

    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ToolchainId {
    fn default() -> Self {
        Self::JAVASCRIPT
    }
}

impl fmt::Display for ToolchainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A package discovered by a toolchain.
///
/// `descriptor` is the toolchain-neutral package descriptor. [`PackageJson`]
/// serves as that descriptor for every toolchain: JavaScript packages parse
/// theirs from disk, while other toolchains synthesize one from their native
/// manifest (only the fields they populate — at minimum `name` and internal
/// dependencies — are meaningful).
#[derive(Debug, Clone)]
pub struct DiscoveredPackage {
    /// The toolchain-neutral package descriptor.
    pub descriptor: PackageJson,
    /// Absolute path to the package's native manifest (`package.json`,
    /// `Cargo.toml`, ...).
    pub manifest_path: AbsoluteSystemPathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Discovery(#[from] discovery::Error),
    #[error(transparent)]
    Descriptor(#[from] crate::package_json::Error),
}

/// The future returned by [`Toolchain::discover_packages`]. Boxed so the
/// trait stays object-safe; toolchains live behind `dyn Toolchain` in the
/// [`ToolchainRegistry`].
pub type DiscoverPackagesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<DiscoveredPackage>, Error>> + Send + 'a>>;

/// A language ecosystem that contributes packages to the repository.
///
/// See the module docs for the design rules trait methods must follow.
pub trait Toolchain: Send + Sync {
    /// This toolchain's identifier.
    fn id(&self) -> ToolchainId;

    /// Discover this toolchain's packages.
    fn discover_packages(&self) -> DiscoverPackagesFuture<'_>;
}

/// The set of toolchains contributing packages to the repository.
///
/// All toolchain lookups go through the registry; it is the single place
/// that knows which toolchains exist. Today entries are registered
/// statically during package graph construction. A future plugin system
/// would construct entries from a manifest instead — an additive change.
#[derive(Default)]
pub struct ToolchainRegistry {
    toolchains: Vec<Arc<dyn Toolchain>>,
}

impl ToolchainRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a toolchain. Registration order is discovery order.
    pub fn register(&mut self, toolchain: Arc<dyn Toolchain>) {
        debug_assert!(
            self.get(&toolchain.id()).is_none(),
            "toolchain {} registered twice",
            toolchain.id()
        );
        self.toolchains.push(toolchain);
    }

    pub fn get(&self, id: &ToolchainId) -> Option<&dyn Toolchain> {
        self.toolchains
            .iter()
            .find(|toolchain| toolchain.id() == *id)
            .map(AsRef::as_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Toolchain> {
        self.toolchains.iter().map(AsRef::as_ref)
    }
}

impl fmt::Debug for ToolchainRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.toolchains.iter().map(|toolchain| toolchain.id()))
            .finish()
    }
}

/// The JavaScript toolchain: packages discovered from `package.json`
/// manifests.
///
/// Wraps a [`PackageDiscovery`] strategy (local filesystem walk,
/// daemon-backed, or a composition) — the strategy decides *how* manifests
/// are found, the toolchain owns *what a JavaScript package is*: it loads
/// and parses each manifest into the package descriptor.
pub struct JavaScriptToolchain<P> {
    discovery: P,
}

impl<P: PackageDiscovery + Send + Sync> JavaScriptToolchain<P> {
    pub fn new(discovery: P) -> Self {
        Self { discovery }
    }

    /// The repository's JavaScript package manager.
    ///
    /// Known debt (see module docs): dependency splitting and lockfile
    /// handling in the package graph builder are not yet trait concerns, so
    /// they reach into the JavaScript toolchain directly for this.
    pub async fn package_manager(&self) -> Result<PackageManager, discovery::Error> {
        Ok(self.discovery.discover_packages().await?.package_manager)
    }
}

impl<P: PackageDiscovery + Send + Sync> Toolchain for JavaScriptToolchain<P> {
    fn id(&self) -> ToolchainId {
        ToolchainId::JAVASCRIPT
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            let workspaces = self.discovery.discover_packages().await?.workspaces;
            // Parse manifests in parallel; manifest parsing dominates
            // discovery time on large repositories.
            turborepo_rayon_compat::block_in_place(|| {
                use rayon::prelude::*;
                workspaces
                    .into_par_iter()
                    .map(|workspace| {
                        let descriptor = PackageJson::load(&workspace.package_json)?;
                        Ok(DiscoveredPackage {
                            descriptor,
                            manifest_path: workspace.package_json,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toolchain_id_is_open() {
        let js = ToolchainId::default();
        assert_eq!(js, ToolchainId::JAVASCRIPT);
        assert_eq!(js.as_str(), "javascript");

        // Any string is a valid id; no closed set to extend.
        let custom = ToolchainId::new("cargo");
        assert_ne!(custom, js);
        assert_eq!(custom.to_string(), "cargo");
        let dynamic = ToolchainId::new(String::from("python-uv"));
        assert_eq!(dynamic.as_str(), "python-uv");
    }

    #[test]
    fn test_registry_lookup() {
        struct Fake(ToolchainId);
        impl Toolchain for Fake {
            fn id(&self) -> ToolchainId {
                self.0.clone()
            }
            fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
                Box::pin(async { Ok(Vec::new()) })
            }
        }

        let mut registry = ToolchainRegistry::new();
        registry.register(Arc::new(Fake(ToolchainId::JAVASCRIPT)));
        registry.register(Arc::new(Fake(ToolchainId::new("cargo"))));

        assert!(registry.get(&ToolchainId::JAVASCRIPT).is_some());
        assert!(registry.get(&ToolchainId::new("cargo")).is_some());
        assert!(registry.get(&ToolchainId::new("zig")).is_none());
        assert_eq!(registry.iter().count(), 2);
    }
}
