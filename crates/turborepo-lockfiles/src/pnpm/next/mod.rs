//! pnpm lockfile v13 (`lockfileVersion: '13.0'`, pnpm/rfcs#72).

mod emit;
mod implied_version;
mod key;
mod prune;
mod read;
mod restore;
mod tarjan;
mod wire;

use std::sync::Arc;

use serde_yaml_ng::Value;
pub(super) use wire::declares_next_format;

use super::{Format, PnpmLockfile};
use crate::pnpm::{LockfileVersion, NextFormatError, VersionFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NextLockfile {
    /// Kept as text because pnpm copies the env document verbatim.
    env: Option<String>,
    tree: Value,
}

impl NextLockfile {
    pub(super) fn read(bytes: &[u8]) -> Result<PnpmLockfile, crate::Error> {
        let split = wire::split(bytes)?;
        let tree = wire::parse_main(&split.main)?;
        let leading_documents = split
            .env
            .as_deref()
            .map(serde_yaml_ng::from_str::<Value>)
            .transpose()?
            .into_iter()
            .collect();
        NextLockfile {
            env: split.env,
            tree,
        }
        .model(leading_documents)
    }

    pub(super) fn prune(&self, kept: &PnpmLockfile) -> Result<PnpmLockfile, crate::Error> {
        NextLockfile {
            env: self.env.clone(),
            tree: prune::prune(&self.tree, kept)?,
        }
        .model(kept.leading_documents.clone())
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        emit::encode(self.env.as_deref(), &self.tree).into_bytes()
    }

    fn model(self, leading_documents: Vec<Value>) -> Result<PnpmLockfile, crate::Error> {
        let (lockfile_version, sections) = {
            let doc = wire::Document::parse(&self.tree)?;
            let version = doc
                .root
                .get("lockfileVersion")
                .and_then(Value::as_str)
                .ok_or_else(|| wire::syntax("lockfileVersion is not a string"))?
                .to_string();
            (version, restore::sections(&restore::Resolved::of(&doc)?)?)
        };
        let mut lockfile = PnpmLockfile {
            lockfile_version: LockfileVersion {
                version: lockfile_version,
                format: VersionFormat::String,
            },
            cached_version: Default::default(),
            leading_documents,
            settings: sections.settings,
            catalogs: sections.catalogs,
            pnpmfile_checksum: sections.pnpmfile_checksum,
            never_built_dependencies: None,
            only_built_dependencies: None,
            ignored_optional_dependencies: sections.ignored_optional_dependencies,
            overrides: sections.overrides,
            package_extensions_checksum: sections.package_extensions_checksum,
            patched_dependencies: sections.patched_dependencies,
            specifiers: None,
            dependencies: None,
            optional_dependencies: None,
            dev_dependencies: None,
            root_dependencies_meta: None,
            importers: sections.importers,
            packages: sections.packages,
            snapshots: sections.snapshots,
            dependency_index: Default::default(),
            time: sections.time,
            format: Format::Next(Arc::new(self)),
        };
        lockfile.cached_version = lockfile.compute_version();
        lockfile.build_dependency_index();
        Ok(lockfile)
    }
}

impl From<NextFormatError> for crate::Error {
    fn from(error: NextFormatError) -> Self {
        crate::Error::Pnpm(error.into())
    }
}

#[cfg(test)]
mod tests;
