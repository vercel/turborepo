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
    #[error(
        "Unsupported pnpm lockfile version {0}. Turborepo reads pnpm lockfiles up to version 9.0. \
         If you enabled pnpm's experimental `lockfile.format: v13`, remove it (or set it to `v9`) \
         and run `pnpm install --fix-lockfile`."
    )]
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

/// The newest lockfile major version this parser understands. pnpm 9 through
/// 12 write `9.0` by default; pnpm 12.20's experimental `lockfile.format: v13`
/// writes `13.0` with a different layout. Reading a newer version with the v6
/// layout (the fallback for unknown string versions) would only fail with an
/// unrelated serde error, or worse, succeed with a wrong graph.
const NEWEST_SUPPORTED_MAJOR: u32 = 9;

impl LockfileVersion {
    fn ensure_supported(&self) -> Result<(), Error> {
        if !matches!(self.format, VersionFormat::String) {
            return Ok(());
        }
        let major = self
            .version
            .split('.')
            .next()
            .and_then(|major| major.parse::<u32>().ok());
        match major {
            Some(major) if major > NEWEST_SUPPORTED_MAJOR => {
                Err(Error::UnsupportedVersion(self.version.clone()))
            }
            _ => Ok(()),
        }
    }
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
