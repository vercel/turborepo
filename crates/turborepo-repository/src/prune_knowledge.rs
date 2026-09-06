//! Immutable, generation-owned knowledge used to plan and finalize native
//! prune output.

use std::{borrow::Cow, collections::BTreeMap, fmt::Debug, sync::Arc};

use turbopath::AbsoluteSystemPath;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PruneDomainId(Cow<'static, str>);

impl PruneDomainId {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }
}

impl std::fmt::Display for PruneDomainId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub const CARGO_PRUNE_DOMAIN: PruneDomainId = PruneDomainId(Cow::Borrowed("cargo"));

pub const PYTHON_PRUNE_DOMAIN: PruneDomainId = PruneDomainId(Cow::Borrowed("python"));

pub const GO_PRUNE_DOMAIN: PruneDomainId = PruneDomainId(Cow::Borrowed("go"));

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
    #[error("duplicate prune domain {0}")]
    DuplicateDomain(PruneDomainId),
    #[error("scope references unavailable prune domain {0}")]
    MissingDomain(PruneDomainId),
}

/// Immutable discovery output capable of projecting and finalizing a prune.
///
/// Implementations contain only data captured for one repository generation;
/// they are not live toolchains and cannot mutate discovery authority.
pub trait PruneDomain: Debug + Send + Sync {
    fn id(&self) -> &PruneDomainId;
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
    domains: BTreeMap<PruneDomainId, Arc<dyn PruneDomain>>,
}

impl PruneKnowledge {
    pub(crate) fn new(
        domains: Vec<Arc<dyn PruneDomain>>,
        referenced: impl IntoIterator<Item = PruneDomainId>,
    ) -> Result<Self, Error> {
        let mut retained = BTreeMap::new();
        for domain in domains {
            let replaced = retained.insert(domain.id().clone(), domain);
            if let Some(replaced) = replaced {
                return Err(Error::DuplicateDomain(replaced.id().clone()));
            }
        }
        for domain in referenced {
            if !retained.contains_key(&domain) {
                return Err(Error::MissingDomain(domain));
            }
        }
        Ok(Self { domains: retained })
    }

    pub fn domains(&self) -> impl Iterator<Item = &PruneDomainId> {
        self.domains.keys()
    }

    pub fn plan(
        &self,
        domain: &PruneDomainId,
        kept_packages: &[String],
    ) -> Result<Option<PrunePlan>, Error> {
        self.domains
            .get(domain)
            .map(|domain| domain.plan(kept_packages))
            .transpose()
            .map(Option::flatten)
    }

    pub fn finalize(
        &self,
        domain: &PruneDomainId,
        pruned_root: &AbsoluteSystemPath,
    ) -> Vec<String> {
        self.domains
            .get(domain)
            .map_or_else(Vec::new, |domain| domain.finalize(pruned_root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeDomain(PruneDomainId);

    impl PruneDomain for FakeDomain {
        fn id(&self) -> &PruneDomainId {
            &self.0
        }

        fn plan(&self, _kept_packages: &[String]) -> Result<Option<PrunePlan>, Error> {
            Ok(None)
        }
    }

    #[test]
    fn rejects_duplicate_and_missing_domains() {
        let domain = PruneDomainId::new("native");
        let duplicate = PruneKnowledge::new(
            vec![
                Arc::new(FakeDomain(domain.clone())),
                Arc::new(FakeDomain(domain.clone())),
            ],
            [],
        );
        assert!(matches!(duplicate, Err(Error::DuplicateDomain(id)) if id == domain));

        let missing = PruneKnowledge::new(
            vec![Arc::new(FakeDomain(domain))],
            [PruneDomainId::new("missing")],
        );
        assert!(
            matches!(missing, Err(Error::MissingDomain(id)) if id == PruneDomainId::new("missing"))
        );
    }
}
