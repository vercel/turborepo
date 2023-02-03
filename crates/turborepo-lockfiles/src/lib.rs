mod error;
mod npm;

use std::collections::{HashMap, HashSet};

pub use error::Error;
pub use npm::*;

#[derive(Debug, PartialEq, Eq, Clone, PartialOrd, Ord, Hash)]
pub struct Package {
    pub key: String,
    pub version: String,
}

// this should get replaced by petgraph in the future :)
pub fn transitive_closure(
    lockfile: &NpmLockfile,
    workspace_path: String,
    unresolved_deps: HashMap<String, String>,
) -> Result<HashSet<Package>, Error> {
    let mut transitive_deps = HashSet::new();
    transitive_closure_helper(
        lockfile,
        &workspace_path,
        unresolved_deps,
        &mut transitive_deps,
    )?;

    Ok(transitive_deps)
}

fn transitive_closure_helper(
    lockfile: &NpmLockfile,
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
