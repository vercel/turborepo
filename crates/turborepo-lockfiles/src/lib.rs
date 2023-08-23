#![deny(clippy::all)]

mod berry;
mod error;
mod npm;
mod pnpm;
mod yarn1;

use std::collections::{HashMap, HashSet};

pub use berry::{Error as BerryError, *};
pub use error::Error;
pub use npm::*;
pub use pnpm::{pnpm_global_change, pnpm_subgraph, PnpmLockfile};
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
pub trait Lockfile {
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

    /// Present a global change key which is compared against two lockfiles
    ///
    /// Impl notes: please prefix this key with some magic identifier
    /// to prevent clashes. we are not worried about inter-version
    /// compatibility so these keys don't need to be stable. They are
    /// ephemeral.
    fn global_change_key(&self) -> Vec<u8>;
}

/// Takes a lockfile, and a map of workspace directory paths -> (package name,
/// version) and calculates the transitive closures for all of them
pub fn all_transitive_closures<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspaces: HashMap<String, HashMap<String, String>>,
) -> Result<HashMap<String, HashSet<Package>>, Error> {
    workspaces
        .into_iter()
        .map(|(workspace, unresolved_deps)| {
            let closure = transitive_closure(lockfile, &workspace, unresolved_deps)?;
            Ok((workspace, closure))
        })
        .collect()
}

// this should get replaced by petgraph in the future :)
pub fn transitive_closure<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: HashMap<String, String>,
) -> Result<HashSet<Package>, Error> {
    let mut transitive_deps = HashSet::new();
    transitive_closure_helper(
        lockfile,
        workspace_path,
        unresolved_deps,
        &mut transitive_deps,
    )?;

    Ok(transitive_deps)
}

fn transitive_closure_helper<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: HashMap<String, impl AsRef<str>>,
    resolved_deps: &mut HashSet<Package>,
) -> Result<(), Error> {
    for (name, specifier) in unresolved_deps {
        let pkg = lockfile.resolve_package(workspace_path, &name, specifier.as_ref())?;

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
                    transitive_closure_helper(lockfile, workspace_path, deps, resolved_deps)?;
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
