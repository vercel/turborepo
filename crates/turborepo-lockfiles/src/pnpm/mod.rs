mod data;
mod de;
mod dep_path;
mod ser;

pub use data::{pnpm_global_change, PnpmLockfile};

use crate::Lockfile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error parsing dependency path: {0}")]
    DependencyPath(#[from] dep_path::Error),
    #[error("Unable to find '{0}' other than reference in dependenciesMeta")]
    MissingInjectedPackage(String),
    #[error("unsupported lockfile version: {0}")]
    UnsupportedVersion(String),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedLockfileVersion {
    V5,
    V6,
    // As of pnpm@9.0.0-rc.0 the lockfile version will now match the pnpm version
    // Lockfile version 7.0 and 9.0 are both the same version
    // See https://github.com/pnpm/pnpm/pull/7861
    V7AndV9,
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
