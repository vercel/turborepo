//! Read-only strict Node/pnpm authorization prerequisite, not managed-run
//! wiring. The caller owns exact root discovery, feature gating, pre-handoff
//! compatibility, execution-context qualification, hashing, and task
//! environment propagation. Do not extract this interface independently of the
//! producer's private schema.

use std::path::{Path, PathBuf};

use turborepo_setup::{
    activation::ActivationPlan, execution_identity::ExecutionContext, lock::Snapshot,
};

use super::trust::TrustStore;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("managed installation has no matching trusted setup record; run turbo setup")]
    Untrusted,
    #[error("managed installation changed after selection; run turbo setup")]
    Changed,
    #[error(transparent)]
    Plan(#[from] turborepo_setup::activation::Error),
    #[error(
        "cannot validate managed setup sources; run turbo setup locally and commit the changed \
         turbo.lock: {0}"
    )]
    Sources(#[from] turborepo_setup::lock::StorageError),
    #[error("cannot validate private setup user state; run turbo setup: {0}")]
    State(#[from] std::io::Error),
}

/// Unlike ActivationPlan, this selection requires external setup evidence.
/// No public plan accessor: the intended use goes through fresh validation.
/// The callback can copy plan data out; the type does NOT enforce that those
/// copies are used immediately or revalidated later. Consumers must call
/// with_current at each actual probe/dispatch boundary. This is not a
/// filesystem lease against same-user hostile mutation after validation.
// Deliberately unwired until managed-run gating/handoff/hashing land together.
#[allow(dead_code)]
pub(crate) struct Authorization {
    root: PathBuf,
    sources: Snapshot,
    context: ExecutionContext,
    plan: ActivationPlan,
    trust: TrustStore,
}

#[allow(dead_code)]
impl Authorization {
    /// User-state base must come from the caller's user directory, never config
    /// authored by the repository. Missing state fails closed without creating
    /// it.
    pub(crate) fn inspect(
        root: &Path,
        user_state_base: &Path,
        context: ExecutionContext,
    ) -> Result<Self, Error> {
        let root = root.canonicalize()?;
        let sources = Snapshot::capture(&root)?;
        let trust = TrustStore::new(user_state_base, &root)?;
        trust.preflight()?;
        let plan = ActivationPlan::inspect(&root, context.clone())?;
        if !trust.authorizes(&plan)? {
            return Err(Error::Untrusted);
        }
        sources.ensure_current()?;
        Ok(Self {
            root,
            sources,
            context,
            plan,
            trust,
        })
    }

    /// Invoke only at the actual use boundary, not to cache PATH for later use.
    /// Revalidate original root/source bytes, complete installation trees and
    /// exports, exact generation, and external evidence before invoking the
    /// callback. No resolution, network, writes, locks, probes or task
    /// execution.
    pub(crate) fn with_current<T>(
        &self,
        use_plan: impl FnOnce(&ActivationPlan) -> T,
    ) -> Result<T, Error> {
        self.sources.ensure_current()?;
        let current = ActivationPlan::inspect(&self.root, self.context.clone())?;
        if current.path_prepend() != self.plan.path_prepend()
            || current.tools() != self.plan.tools()
            || current.snapshot().canonical_identity() != self.plan.snapshot().canonical_identity()
        {
            return Err(Error::Changed);
        }
        if !self.trust.authorizes(&current)? {
            return Err(Error::Untrusted);
        }
        self.sources.ensure_current()?;
        Ok(use_plan(&current))
    }
}
