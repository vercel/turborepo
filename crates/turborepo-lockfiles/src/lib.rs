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
mod error;
mod npm;
mod pnpm;
mod yarn1;

use std::{
    any::Any,
    borrow::Cow,
    collections::{HashMap, HashSet},
};

pub use berry::{Error as BerryError, *};
pub use bun::{BunLockfile, bun_global_change};
use dashmap::DashMap;
pub use error::Error;
pub use npm::*;
pub use pnpm::{PnpmLockfile, pnpm_global_change, pnpm_subgraph};
use rayon::prelude::*;
use serde::Serialize;
use turbopath::RelativeUnixPathBuf;
pub use yarn1::{Yarn1Lockfile, yarn_subgraph};

type ResolveCache = DashMap<String, Option<Package>>;

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
    ) -> Result<Option<Cow<'_, HashMap<String, String>>>, Error>;

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
}

/// Takes a lockfile, and a map of workspace directory paths -> (package name,
/// version) and calculates the transitive closures for all of them
pub fn all_transitive_closures<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspaces: HashMap<String, HashMap<String, String>>,
    ignore_missing_packages: bool,
) -> Result<HashMap<String, HashSet<Package>>, Error> {
    let resolve_cache: ResolveCache = DashMap::new();
    workspaces
        .into_par_iter()
        .map(|(workspace, unresolved_deps)| {
            let closure = transitive_closure_cached(
                lockfile,
                &workspace,
                unresolved_deps,
                ignore_missing_packages,
                &resolve_cache,
            )?;
            Ok((workspace, closure))
        })
        .collect()
}

#[tracing::instrument(skip_all)]
pub fn transitive_closure<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: HashMap<String, String>,
    ignore_missing_packages: bool,
) -> Result<HashSet<Package>, Error> {
    let resolve_cache: ResolveCache = DashMap::new();
    transitive_closure_cached(
        lockfile,
        workspace_path,
        unresolved_deps,
        ignore_missing_packages,
        &resolve_cache,
    )
}

fn transitive_closure_cached<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: HashMap<String, String>,
    ignore_missing_packages: bool,
    resolve_cache: &ResolveCache,
) -> Result<HashSet<Package>, Error> {
    let mut ctx = ClosureContext {
        lockfile,
        workspace_path,
        resolve_cache,
        key_buf: String::new(),
    };
    let mut transitive_deps = HashSet::new();
    ctx.walk(
        &unresolved_deps,
        &mut transitive_deps,
        ignore_missing_packages,
        true,
    )?;
    Ok(transitive_deps)
}

struct ClosureContext<'a, L: Lockfile + ?Sized> {
    lockfile: &'a L,
    workspace_path: &'a str,
    resolve_cache: &'a ResolveCache,
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
        unresolved_deps: &HashMap<String, String>,
        ignore_missing_packages: bool,
        is_workspace_root_deps: bool,
    ) -> Result<Vec<Package>, Error> {
        let mut newly_resolved = Vec::new();

        for (name, specifier) in unresolved_deps {
            // For direct workspace dependencies, include workspace_path in the cache key
            // since resolution depends on the workspace's importer entry.
            // For transitive sub-dependencies, the resolution is workspace-independent
            // (the version is already a resolved lockfile key), so we omit workspace_path
            // to enable cross-workspace cache sharing.
            let wp = is_workspace_root_deps.then_some(self.workspace_path);
            self.make_cache_key(wp, name, specifier);

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
                    self.resolve_cache
                        .insert(self.key_buf.clone(), result.clone());
                    result
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
        unresolved_deps: &HashMap<String, String>,
        resolved_deps: &mut HashSet<Package>,
        ignore_missing_packages: bool,
        is_workspace_root_deps: bool,
    ) -> Result<(), Error> {
        let newly_resolved = self.resolve_deps(
            unresolved_deps,
            ignore_missing_packages,
            is_workspace_root_deps,
        )?;

        for pkg in newly_resolved {
            if resolved_deps.contains(&pkg) {
                continue;
            }

            let all_deps = self.lockfile.all_dependencies(&pkg.key)?;
            resolved_deps.insert(pkg);
            if let Some(deps) = all_deps {
                self.walk(&deps, resolved_deps, false, false)?;
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
