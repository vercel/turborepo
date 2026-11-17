mod data;
mod de;
mod dep_path;
mod ser;

pub use data::{PnpmLockfile, pnpm_global_change};

use crate::Lockfile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Error parsing dependency path: {0}")]
    DependencyPath(#[from] dep_path::Error),
    #[error("Unable to find '{0}' other than reference in `dependenciesMeta`")]
    MissingInjectedPackage(String),
    #[error("Unsupported lockfile version: {0}")]
    UnsupportedVersion(String),
    #[error(
        "Root `{section}.{name}` entry does not match the dependency format of lockfileVersion \
         {version}"
    )]
    RootDependencyShape {
        section: &'static str,
        name: String,
        version: String,
    },
    #[error(transparent)]
    NextFormat(#[from] NextFormatError),
}

/// Mirrors pnpm's `NextFormatError` where the cause is the same.
#[derive(Debug, thiserror::Error)]
pub enum NextFormatError {
    #[error("Unable to parse pnpm v13 lockfile: {0}")]
    Syntax(String),
    #[error(
        "The lockfile does not say which version {name} resolves to in {importer}: nothing \
         follows from the specifier {specifier}"
    )]
    UnderivableVersion {
        importer: String,
        name: String,
        specifier: String,
    },
    #[error(
        "The lockfile does not say which version {name} resolves to in the catalog {catalog}: \
         nothing follows from the specifier {specifier}"
    )]
    UnderivableCatalogVersion {
        catalog: String,
        name: String,
        specifier: String,
    },
    #[error("The lockfile records no resolution for {key}")]
    MissingResolution { key: String },
    #[error("Two snapshots of the pnpm v13 lockfile restore to the same key {key}")]
    DuplicateKey { key: String },
    #[error("Pruning retained {key}, which no snapshot of the pnpm v13 lockfile restores to")]
    UnknownRetainedKey { key: String },
    #[error("Unable to write the pruned pnpm v13 lockfile: {detail}")]
    NotRepresentable { detail: String },
    #[error(
        "Unable to prune a pnpm v13 lockfile merged from per-project lockfiles \
         (shared-workspace-lockfile=false)"
    )]
    Merged,
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum SupportedLockfileVersion {
    V5,
    #[default]
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
