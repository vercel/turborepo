#![feature(trait_upcasting)]
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
pub use bun::BunLockfile;
pub use error::Error;
pub use npm::*;
pub use pnpm::{pnpm_global_change, pnpm_subgraph, PnpmLockfile};
use rayon::prelude::*;
use serde::Serialize;
use turbopath::RelativeUnixPathBuf;
pub use yarn1::{yarn_subgraph, Yarn1Lockfile};

#[derive(Debug, PartialEq, Eq, Clone, PartialOrd, Ord, Hash, Serialize)]
pub struct Package {
    pub key: String,
    pub version: String,
}

// This trait will only be used when migrating the Go lockfile implementations
// to Rust. Once the migration is complete we will leverage petgraph for doing
// our graph calculations.
pub trait Lockfile: Send + Sync + Any + std::fmt::Debug {
    // Given a workspace, a package it imports and version returns the key, resolved
    // version, and if it was found
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<Package>, Error>;
    // Given a lockfile key return all (prod/dev/optional) dependencies of that
    // package
    fn all_dependencies(&self, key: &str) -> Result<Option<HashMap<String, String>>, Error>;

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, Error>;

    fn encode(&self) -> Result<Vec<u8>, Error>;

    /// All patch files referenced in the lockfile
    fn patches(&self) -> Result<Vec<RelativeUnixPathBuf>, Error> {
        Ok(Vec::new())
    }

    /// Determine if there's a global change between two lockfiles
    fn global_change(&self, other: &dyn Lockfile) -> bool;

    /// Return any turbo version found in the lockfile
    fn turbo_version(&self) -> Option<String>;
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

// this should get replaced by petgraph in the future :)
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
                    transitive_closure_helper(
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
