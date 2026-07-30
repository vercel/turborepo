//! Immutable, generation-owned knowledge used to plan and finalize native
//! prune output.

use std::{collections::BTreeMap, fmt::Debug, sync::Arc};

use turbopath::AbsoluteSystemPath;

use crate::toolchain::ToolchainId;

/// A toolchain's contribution to a pruned repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    /// Packages that must additionally be retained and copied.
    pub extra_packages: Vec<String>,
    /// Files to write as `(repo-relative unix path, contents)`.
    pub root_files: Vec<(String, String)>,
    /// Repo-relative unix paths to copy verbatim when present.
    pub copy_paths: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An ecosystem-specific planning failure.
    #[error(transparent)]
    Failed(Box<dyn std::error::Error + Send + Sync>),
}

/// Immutable discovery output capable of projecting and finalizing a prune.
///
/// Implementations contain only data captured for one repository generation;
/// they are not live toolchains and cannot mutate discovery authority.
pub trait PruneDomain: Debug + Send + Sync {
    fn toolchain(&self) -> &ToolchainId;
    fn plan(&self, kept_packages: &[String]) -> Result<Option<PrunePlan>, Error>;

    /// Polish a fully materialized prune and return the repo-relative files
    /// changed so alternate output layers can synchronize them.
    fn finalize(&self, pruned_root: &AbsoluteSystemPath) -> Vec<String> {
        let _ = pruned_root;
        Vec::new()
    }
}

/// All native prune domains retained by a package-graph generation. Runtime
/// orchestration never consults the live discovery producer.
#[derive(Debug, Default)]
pub struct PruneKnowledge {
    domains: BTreeMap<ToolchainId, Arc<dyn PruneDomain>>,
}

impl PruneKnowledge {
    pub(crate) fn new(domains: Vec<Arc<dyn PruneDomain>>) -> Self {
        let mut retained = BTreeMap::new();
        for domain in domains {
            let replaced = retained.insert(domain.toolchain().clone(), domain);
            debug_assert!(replaced.is_none(), "duplicate prune knowledge domain");
        }
        Self { domains: retained }
    }

    pub fn toolchains(&self) -> impl Iterator<Item = &ToolchainId> {
        self.domains.keys()
    }

    pub fn plan(
        &self,
        toolchain: &ToolchainId,
        kept_packages: &[String],
    ) -> Result<Option<PrunePlan>, Error> {
        self.domains
            .get(toolchain)
            .map(|domain| domain.plan(kept_packages))
            .transpose()
            .map(Option::flatten)
    }

    pub fn finalize(
        &self,
        toolchain: &ToolchainId,
        pruned_root: &AbsoluteSystemPath,
    ) -> Vec<String> {
        self.domains
            .get(toolchain)
            .map_or_else(Vec::new, |domain| domain.finalize(pruned_root))
    }
}
