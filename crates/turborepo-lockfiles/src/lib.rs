//! Package manager lockfile parsing, analysis, and serialization
//!
//! Parsing and analysis are used to track which external packages a workspace
//! package depends on. This allows Turborepo to not perform a global
//! invalidation on a lockfile change, but instead only the packages which
//! depend on the changed external packages.
//!
//! Serialization is exclusively used by `turbo prune` and is far more error
//! prone than deserialization and analysis.

#![deny(clippy::all)]
// the pest proc macro adds an empty doc comment.
#![allow(clippy::empty_docs)]

mod berry;
mod bun;
mod cargo;
mod closure_dp;
mod error;
mod npm;
mod pnpm;
mod yarn1;

use std::{
    any::Any,
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

pub use berry::{Error as BerryError, *};
pub use bun::{BunLockfile, bun_global_change};
pub use cargo::{Error as CargoLockError, cargo_external_closures};
use dashmap::DashMap;
pub use error::Error;
pub use npm::*;
pub use pnpm::{PnpmLockfile, pnpm_global_change, pnpm_subgraph};
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashSet};
use serde::Serialize;
use turbopath::RelativeUnixPathBuf;
pub use yarn1::{Yarn1Lockfile, yarn_subgraph};

// The closure walk visits every edge of every workspace's dependency closure,
// so these caches are probed millions of times on large repos. They use
// FxHash instead of the DoS-resistant default: lockfile contents are developer
// tooling input, not an adversarial hash-flooding vector, and the long string
// keys make SipHash a measurable cost.
//
// Resolutions cache interned package ids (plus a shared `Arc` of the package)
// so a cache hit is a refcount bump instead of cloning the package's strings.
type ResolveCache = DashMap<String, Option<(u32, Arc<Package>)>, FxBuildHasher>;
// Dependency maps are shared behind an `Arc` so cache hits are a refcount bump
// instead of a deep clone of the map. The cache is shared across all
// workspaces, so each package's dependency map would otherwise be cloned once
// per workspace whose closure contains it. Keyed by interned package id to
// avoid re-hashing package key strings on every node visit.
type DepsCache = DashMap<u32, Option<Arc<BTreeMap<String, String>>>, FxBuildHasher>;

/// Assigns dense integer ids to resolved packages. Distinct cache keys (e.g.
/// the same package reached from different workspaces) intern to the same id,
/// letting visited-set checks hash a `u32` instead of two heap strings.
#[derive(Default)]
struct PackageInterner {
    ids: DashMap<Package, (u32, Arc<Package>), FxBuildHasher>,
    next: std::sync::atomic::AtomicU32,
}

impl PackageInterner {
    fn intern(&self, package: Package) -> (u32, Arc<Package>) {
        match self.ids.entry(package) {
            dashmap::mapref::entry::Entry::Occupied(entry) => entry.get().clone(),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let package = Arc::new(entry.key().clone());
                entry.insert((id, package.clone()));
                (id, package)
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, PartialOrd, Ord, Hash, Serialize)]
pub struct Package {
    pub key: String,
    pub version: String,
}

/// A trait for exposing common operations for lockfile parsing, analysis, and
/// encoding.
///
/// External packages are identified by key strings which have no shared
/// structure other than being able to uniquely identify a package in the
/// corresponding lockfile. When programming against these keys they should be
/// viewed as a black box and any logic for handling them should live in the
/// specific lockfile implementation which might have additional understanding
/// of them. Using `human_name` can provide a version of the key that is
/// formatted for human viewing. The fact that keys are still represented as
/// `String`s is a vestige of the translation from Go.
///
/// We cannot easily expose lockfiles as a standard
/// graph due to overrides that various lockfile formats support. A dependency
/// of `"package": "1.0.0"` might resolve to a different version depending on
/// how it is imported. See https://pnpm.io/settings#overrides
pub trait Lockfile: Send + Sync + Any + std::fmt::Debug {
    /// Resolve a dependency declaration from a workspace package to a lockfile
    /// key
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<Package>, Error>;

    /// Given a lockfile key return all (prod/dev/optional) direct dependencies
    /// of that package.
    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<Cow<'_, BTreeMap<String, String>>>, Error>;

    /// Given a list of workspace packages and external packages that are
    /// dependencies of the workspace packages, produce a lockfile that only
    /// references said packages.
    ///
    /// The caller is expected to have calculated the correct `packages` for the
    /// provided `workspace_packages` using `resolve_package` and
    /// `all_dependencies` as otherwise `subgraph` might fail or produce an
    /// incorrect lockfile.
    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, Error>;

    /// Encode the lockfile to a string of bytes that can be written to disk
    fn encode(&self) -> Result<Vec<u8>, Error>;

    /// All patch files referenced in the lockfile
    ///
    /// Useful for identifying any patch files that are referenced by the
    /// lockfile
    fn patches(&self) -> Result<Vec<RelativeUnixPathBuf>, Error> {
        Ok(Vec::new())
    }

    /// Package identifiers that have patches in the lockfile.
    ///
    /// Some package managers store patch paths outside of the lockfile, so
    /// these keys are used to recover the relevant paths from package
    /// manager config.
    fn patch_keys(&self) -> Vec<String> {
        Vec::new()
    }

    /// Determine if there's a global change between two lockfiles
    ///
    /// This generally is only `true` across lockfile version changes or when a
    /// setting changes where it is safer to view everything as changed rather
    /// than try to understand the change.
    fn global_change(&self, other: &dyn Lockfile) -> bool;

    /// Return any turbo version found in the lockfile
    ///
    /// Used for identifying which version of `turbo` the lockfile references if
    /// no local `turbo` binary is found.
    fn turbo_version(&self) -> Option<String>;

    /// A human friendly version of a lockfile key.
    /// Usually of the form `package@version`, but version might include
    /// additional information to convey difference from other packages in
    /// the lockfile e.g. differing peer dependencies.
    #[allow(unused)]
    fn human_name(&self, package: &Package) -> Option<String> {
        None
    }

    /// A resolver for transitive dependency edges whose resolution can be
    /// proven identical across workspaces, enabling the shared closure DP
    /// in [`all_transitive_closures`]. `None` (the default) means the
    /// format cannot make that promise and closures use the per-workspace
    /// walk.
    fn transitive_edge_resolver(&self) -> Option<Box<dyn TransitiveEdgeResolver + '_>> {
        None
    }
}

/// Outcome of resolving a transitive `(name, version)` edge without a
/// workspace context.
pub enum TransitiveEdgeResolution {
    /// Every workspace resolves this edge to the same package (or fails to
    /// resolve it).
    Global(Option<Package>),
    /// Some workspace could resolve this edge differently; the shared DP
    /// must not be used.
    WorkspaceSensitive,
}

/// See [`Lockfile::transitive_edge_resolver`].
pub trait TransitiveEdgeResolver {
    fn resolve_edge(&self, name: &str, version: &str) -> Result<TransitiveEdgeResolution, Error>;
}

/// Takes a lockfile, and a map of workspace directory paths -> (package name,
/// version) and calculates the transitive closures for all of them
pub fn all_transitive_closures<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspaces: HashMap<String, BTreeMap<String, String>>,
    ignore_missing_packages: bool,
) -> Result<HashMap<String, HashSet<Package>>, Error> {
    Ok(
        all_transitive_closures_sorted(lockfile, workspaces, ignore_missing_packages)?
            .into_iter()
            .map(|(ws, closure)| (ws, closure.into_iter().map(|pkg| (*pkg).clone()).collect()))
            .collect(),
    )
}

/// Per-workspace transitive closures keyed by workspace directory, each
/// sorted by `Package`'s `(key, version)` ordering with members shared via
/// `Arc`.
pub type SortedClosures = HashMap<String, Vec<Arc<Package>>>;

/// Like [`all_transitive_closures`], but each closure is a `Vec` sorted by
/// `Package`'s `(key, version)` ordering with members shared via `Arc`.
/// The sorted form lets consumers hash or diff closures without re-sorting,
/// and the `Arc` sharing avoids cloning two `String`s per closure member per
/// workspace (shared dependencies are the overwhelming majority in a
/// monorepo).
pub fn all_transitive_closures_sorted<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspaces: HashMap<String, BTreeMap<String, String>>,
    ignore_missing_packages: bool,
) -> Result<SortedClosures, Error> {
    // Shared DP fast path: computes each closure once over the global
    // package graph instead of once per workspace. Only sound when the
    // lockfile proves every transitive edge resolves identically across
    // workspaces; any sensitive edge falls through to the walk below.
    if workspaces.len() > 1
        && let Some(resolver) = lockfile.transitive_edge_resolver()
        && let Some(closures) = closure_dp::all_transitive_closures_dp(
            lockfile,
            resolver.as_ref(),
            &workspaces,
            ignore_missing_packages,
        )?
    {
        return Ok(closures);
    }

    let resolve_cache: ResolveCache = DashMap::default();
    let deps_cache: DepsCache = DashMap::default();
    let interner = PackageInterner::default();
    workspaces
        .into_par_iter()
        .map(|(workspace, unresolved_deps)| {
            let closure = transitive_closure_cached(
                lockfile,
                &workspace,
                unresolved_deps,
                ignore_missing_packages,
                &resolve_cache,
                &deps_cache,
                &interner,
            )?;
            let mut sorted: Vec<Arc<Package>> = closure.into_iter().map(Arc::new).collect();
            sorted.sort_unstable();
            Ok((workspace, sorted))
        })
        .collect()
}

#[tracing::instrument(skip_all)]
pub fn transitive_closure<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: BTreeMap<String, String>,
    ignore_missing_packages: bool,
) -> Result<HashSet<Package>, Error> {
    let resolve_cache: ResolveCache = DashMap::default();
    let deps_cache: DepsCache = DashMap::default();
    let interner = PackageInterner::default();
    transitive_closure_cached(
        lockfile,
        workspace_path,
        unresolved_deps,
        ignore_missing_packages,
        &resolve_cache,
        &deps_cache,
        &interner,
    )
}

#[allow(clippy::too_many_arguments)]
fn transitive_closure_cached<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: BTreeMap<String, String>,
    ignore_missing_packages: bool,
    resolve_cache: &ResolveCache,
    deps_cache: &DepsCache,
    interner: &PackageInterner,
) -> Result<HashSet<Package>, Error> {
    let mut ctx = ClosureContext {
        lockfile,
        workspace_path,
        resolve_cache,
        deps_cache,
        interner,
        key_buf: String::new(),
    };
    let mut transitive_deps = HashSet::new();
    let mut visited = FxHashSet::default();
    ctx.walk(
        &unresolved_deps,
        &mut transitive_deps,
        &mut visited,
        ignore_missing_packages,
        true,
    )?;
    Ok(transitive_deps)
}

struct ClosureContext<'a, L: Lockfile + ?Sized> {
    lockfile: &'a L,
    workspace_path: &'a str,
    resolve_cache: &'a ResolveCache,
    deps_cache: &'a DepsCache,
    interner: &'a PackageInterner,
    key_buf: String,
}

impl<L: Lockfile + ?Sized> ClosureContext<'_, L> {
    fn make_cache_key(&mut self, workspace_path: Option<&str>, name: &str, specifier: &str) {
        self.key_buf.clear();
        if let Some(wp) = workspace_path {
            self.key_buf
                .reserve(wp.len() + name.len() + specifier.len() + 2);
            self.key_buf.push_str(wp);
            self.key_buf.push('\0');
        } else {
            self.key_buf.reserve(name.len() + specifier.len() + 1);
        }
        self.key_buf.push_str(name);
        self.key_buf.push('\0');
        self.key_buf.push_str(specifier);
    }

    fn resolve_deps(
        &mut self,
        unresolved_deps: &BTreeMap<String, String>,
        ignore_missing_packages: bool,
        _is_workspace_root_deps: bool,
    ) -> Result<Vec<(u32, Arc<Package>)>, Error> {
        let mut newly_resolved = Vec::new();

        for (name, specifier) in unresolved_deps {
            // Always include workspace_path in the cache key because
            // resolve_package() receives it and some lockfile implementations
            // (e.g. Bun) use it for workspace-scoped resolution even for
            // transitive dependencies. Without this, parallel workspace
            // processing in all_transitive_closures() can produce
            // non-deterministic results when one workspace's cached resolution
            // is incorrectly reused by another workspace.
            self.make_cache_key(Some(self.workspace_path), name, specifier);

            let pkg = match self.resolve_cache.get(self.key_buf.as_str()) {
                Some(cached) => cached.clone(),
                None => {
                    let result =
                        match self
                            .lockfile
                            .resolve_package(self.workspace_path, name, specifier)
                        {
                            Ok(pkg) => pkg,
                            Err(Error::MissingWorkspace(_)) if ignore_missing_packages => {
                                self.resolve_cache.insert(self.key_buf.clone(), None);
                                continue;
                            }
                            Err(e) => return Err(e),
                        };
                    let interned = result.map(|pkg| self.interner.intern(pkg));
                    self.resolve_cache
                        .insert(self.key_buf.clone(), interned.clone());
                    interned
                }
            };

            if let Some(pkg) = pkg {
                newly_resolved.push(pkg);
            }
        }

        Ok(newly_resolved)
    }

    fn walk(
        &mut self,
        unresolved_deps: &BTreeMap<String, String>,
        resolved_deps: &mut HashSet<Package>,
        visited: &mut FxHashSet<u32>,
        ignore_missing_packages: bool,
        is_workspace_root_deps: bool,
    ) -> Result<(), Error> {
        let newly_resolved = self.resolve_deps(
            unresolved_deps,
            ignore_missing_packages,
            is_workspace_root_deps,
        )?;

        for (id, pkg) in newly_resolved {
            if !visited.insert(id) {
                continue;
            }

            let all_deps = if let Some(cached) = self.deps_cache.get(&id) {
                cached.clone()
            } else {
                let deps = self
                    .lockfile
                    .all_dependencies(&pkg.key)?
                    .map(|cow| Arc::new(cow.into_owned()));
                self.deps_cache.insert(id, deps.clone());
                deps
            };

            resolved_deps.insert((*pkg).clone());
            if let Some(deps) = all_deps {
                self.walk(&deps, resolved_deps, visited, false, false)?;
            }
        }

        Ok(())
    }
}

impl Package {
    pub fn new(key: impl Into<String>, version: impl Into<String>) -> Self {
        let key = key.into();
        let version = version.into();
        Self { key, version }
    }
}
