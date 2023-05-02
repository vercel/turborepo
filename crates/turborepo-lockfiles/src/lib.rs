#![feature(once_cell)]
#![feature(test)]
extern crate test;

mod berry;
mod error;
mod npm;

use std::collections::{HashMap, HashSet};

pub use berry::{Error as BerryError, *};
pub use error::Error;
pub use npm::*;
use rayon::prelude::*;
use serde::Serialize;

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
}

pub fn all_transitive_closures<L: Lockfile + Sync>(
    lockfile: &L,
    workspaces: HashMap<String, HashMap<String, String>>,
) -> Result<HashMap<String, HashSet<Package>>, Error> {
    workspaces
        .into_par_iter()
        .map(|(workspace, unresolved_deps)| {
            let closure = transitive_closure(lockfile, &workspace, unresolved_deps)?;
            Ok((workspace, closure))
        })
        .collect()
}

// this should get replaced by petgraph in the future :)
pub fn transitive_closure<L: Lockfile>(
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

fn transitive_closure_helper<L: Lockfile>(
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

#[cfg(test)]
mod tests {
    use test::Bencher;

    use super::*;

    fn make_workspaces(w: &[(&str, &[(&str, &str)])]) -> HashMap<String, HashMap<String, String>> {
        w.iter()
            .map(|(workspace, deps)| {
                (
                    workspace.to_string(),
                    deps.iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    #[bench]
    fn bench_npm_dependency_crawl(b: &mut Bencher) {
        b.iter(|| {
            let lockfile = NpmLockfile::load(include_bytes!("../fixtures/npm-lock.json")).unwrap();
            all_transitive_closures(
                &lockfile,
                make_workspaces(&[
                    ("", &[("prettier", "latest"), ("turbo", "latest")]),
                    (
                        "apps/docs",
                        &[
                            ("lodash", "^3.0.0"),
                            ("next", "12.3.0"),
                            ("react", "18.2.0"),
                            ("react-dom", "18.2.0"),
                            ("@babel/core", "^7.0.0"),
                            ("@types/node", "^17.0.12"),
                            ("@types/react", "18.0.17"),
                            ("eslint", "7.32.0"),
                            ("next-transpile-modules", "9.0.0"),
                            ("typescript", "^4.5.3"),
                        ],
                    ),
                    (
                        "apps/web",
                        &[
                            ("lodash", "^4.17.21"),
                            ("next", "12.3.0"),
                            ("react", "18.2.0"),
                            ("react-dom", "18.2.0"),
                            ("@babel/core", "^7.0.0"),
                            ("@types/node", "^17.0.12"),
                            ("@types/react", "18.0.17"),
                            ("eslint", "7.32.0"),
                            ("next-transpile-modules", "9.0.0"),
                            ("typescript", "^4.5.3"),
                        ],
                    ),
                    (
                        "packages/eslint-config-custom",
                        &[
                            ("eslint", "^7.23.0"),
                            ("eslint-config-next", "^12.0.8"),
                            ("eslint-config-prettier", "^8.3.0"),
                            ("eslint-config-turbo", "latest"),
                            ("eslint-plugin-react", "7.31.8"),
                            ("typescript", "^4.7.4"),
                        ],
                    ),
                    ("packages/tsconfig", &[]),
                    (
                        "packages/ui",
                        &[
                            ("@types/react", "^17.0.37"),
                            ("@types/react-dom", "^17.0.11"),
                            ("eslint", "^7.32.0"),
                            ("react", "^18.2.0"),
                            ("typescript", "^4.5.2"),
                        ],
                    ),
                ]),
            )
        })
    }

    #[bench]
    fn bench_berry_dependency_crawl(b: &mut Bencher) {
        b.iter(|| {
            let data = LockfileData::from_bytes(include_bytes!("../fixtures/berry.lock")).unwrap();
            let lockfile = BerryLockfile::new(&data, None).unwrap();
            all_transitive_closures(
                &lockfile,
                make_workspaces(&[
                    ("", &[("prettier", "latest"), ("turbo", "latest")]),
                    (
                        "apps/docs",
                        &[
                            ("lodash", "^3.0.0"),
                            ("next", "12.3.0"),
                            ("react", "18.2.0"),
                            ("react-dom", "18.2.0"),
                            ("@babel/core", "^7.0.0"),
                            ("@types/node", "^17.0.12"),
                            ("@types/react", "18.0.17"),
                            ("eslint", "7.32.0"),
                            ("next-transpile-modules", "9.0.0"),
                            ("typescript", "^4.5.3"),
                        ],
                    ),
                    (
                        "apps/web",
                        &[
                            ("lodash", "^4.17.21"),
                            ("next", "12.3.0"),
                            ("react", "18.2.0"),
                            ("react-dom", "18.2.0"),
                            ("@babel/core", "^7.0.0"),
                            ("@types/node", "^17.0.12"),
                            ("@types/react", "18.0.17"),
                            ("eslint", "7.32.0"),
                            ("next-transpile-modules", "9.0.0"),
                            ("typescript", "^4.5.3"),
                        ],
                    ),
                    (
                        "packages/eslint-config-custom",
                        &[
                            ("eslint", "^7.23.0"),
                            ("eslint-config-next", "^12.0.8"),
                            ("eslint-config-prettier", "^8.3.0"),
                            ("eslint-config-turbo", "latest"),
                            ("eslint-plugin-react", "7.31.8"),
                            ("typescript", "^4.7.4"),
                        ],
                    ),
                    ("packages/tsconfig", &[]),
                    (
                        "packages/ui",
                        &[
                            ("@types/react", "^17.0.37"),
                            ("@types/react-dom", "^17.0.11"),
                            ("eslint", "^7.32.0"),
                            ("react", "^18.2.0"),
                            ("typescript", "^4.5.2"),
                        ],
                    ),
                ]),
            )
        })
    }
}
