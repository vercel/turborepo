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
    collections::{HashMap, HashSet},
};

pub use berry::{Error as BerryError, *};
pub use bun::{BunLockfile, bun_global_change};
pub use error::Error;
pub use npm::*;
pub use pnpm::{PnpmLockfile, pnpm_global_change, pnpm_subgraph};
use rayon::prelude::*;
use serde::Serialize;
use turbopath::RelativeUnixPathBuf;
pub use yarn1::{Yarn1Lockfile, yarn_subgraph};

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
    fn all_dependencies(&self, key: &str) -> Result<Option<HashMap<String, String>>, Error>;

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
    workspaces
        .into_par_iter()
        .map(|(workspace, unresolved_deps)| {
            let closure = transitive_closure(
                lockfile,
                &workspace,
                unresolved_deps,
                ignore_missing_packages,
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
    let mut transitive_deps = HashSet::new();
    transitive_closure_helper(
        lockfile,
        workspace_path,
        unresolved_deps,
        &mut transitive_deps,
        ignore_missing_packages,
    )?;

    Ok(transitive_deps)
}

fn transitive_closure_helper<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: HashMap<String, impl AsRef<str>>,
    resolved_deps: &mut HashSet<Package>,
    ignore_missing_packages: bool,
) -> Result<(), Error> {
    transitive_closure_helper_impl(
        lockfile,
        workspace_path,
        unresolved_deps,
        resolved_deps,
        ignore_missing_packages,
    )
}

/// Core transitive closure implementation that walks dependencies.
fn transitive_closure_helper_impl<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: HashMap<String, impl AsRef<str>>,
    resolved_deps: &mut HashSet<Package>,
    ignore_missing_packages: bool,
) -> Result<(), Error> {
    for (name, specifier) in unresolved_deps {
        let pkg = match lockfile.resolve_package(workspace_path, &name, specifier.as_ref()) {
            Ok(pkg) => pkg,
            Err(Error::MissingWorkspace(_)) if ignore_missing_packages => {
                continue;
            }
            Err(e) => return Err(e),
        };

        match pkg {
            None => {
                continue;
            }
            Some(pkg) if resolved_deps.contains(&pkg) => {
                continue;
            }
            Some(pkg) => {
                let all_deps = lockfile.all_dependencies(&pkg.key)?;
                resolved_deps.insert(pkg);
                if let Some(deps) = all_deps {
                    // we've already found one unresolved dependency, so we can't ignore its set of
                    // dependencies.
                    transitive_closure_helper_impl(
                        lockfile,
                        workspace_path,
                        deps,
                        resolved_deps,
                        false,
                    )?;
                }
            }
        }
    }

    Ok(())
}

impl Package {
    pub fn new(key: impl Into<String>, version: impl Into<String>) -> Self {
        let key = key.into();
        let version = version.into();
        Self { key, version }
    }
}
