mod data;
mod de;
mod dep_path;
mod ser;

pub use data::{pnpm_global_change, PnpmLockfile};

use crate::Lockfile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error parsing dependency path: {0}")]
    DependencyPath(#[from] nom::error::Error<String>),
    #[error("Unable to find '{0}' other than reference in dependenciesMeta")]
    MissingInjectedPackage(String),
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct LockfileVersion {
    version: String,
    format: VersionFormat,
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum VersionFormat {
    String,
    Float,
}

pub fn pnpm_subgraph(
    contents: &[u8],
    workspace_packages: &[String],
    packages: &[String],
) -> Result<Vec<u8>, crate::Error> {
    let lockfile = PnpmLockfile::from_bytes(contents)?;
    let pruned_lockfile = lockfile.subgraph(workspace_packages, packages)?;
    let new_contents = pruned_lockfile.encode()?;
    Ok(new_contents)
}
