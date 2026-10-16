//! Read-only strict authorization of turbo.lock or `turbo setup --no-lock`
//! installations, not managed-run wiring. The caller owns exact root
//! discovery, choosing the lock or clone-local mode, feature gating,
//! pre-handoff compatibility, execution-context qualification, hashing, and
//! task environment propagation. Do not extract this interface independently of
//! the producer's private schema.

use std::path::{Path, PathBuf};

use turborepo_setup::{
    activation::ActivationPlan,
    execution_identity::ExecutionContext,
    lock::{Ecosystems, Snapshot},
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

impl Error {
    /// `turbo run` guidance. Says whether turbo setup repairs the
    /// installation or updates turbo.lock, and in CI, where setup is frozen,
    /// sends lock updates back to a local setup. Never recommends a setup
    /// that would fail the same way, such as for an unreadable lock.
    pub(crate) fn guidance(&self, ci: bool) -> String {
        use turborepo_setup::{activation::Error as Plan, lock::StorageError as Storage};
        let repair = |cause: &dyn std::fmt::Display| {
            format!("{cause}; run turbo setup to repair the installation (turbo.lock is unchanged)")
        };
        let invalid = |cause: &dyn std::fmt::Display| {
            format!(
                "turbo.lock is invalid for this repository ({cause}); restore the committed \
                 turbo.lock, or delete it and run turbo setup locally to create a new one, then \
                 commit it"
            )
        };
        match self {
            Self::Plan(Plan::DeclarationDrift) => {
                let fix = if ci {
                    "run turbo setup locally and commit the updated turbo.lock"
                } else {
                    "run turbo setup to update turbo.lock, then commit it"
                };
                format!("tool declarations no longer match turbo.lock; {fix}")
            }
            Self::Plan(Plan::MissingInventory) => repair(&"managed tools are not installed"),
            Self::Plan(Plan::StaleInventory) => repair(&"managed tools do not match turbo.lock"),
            Self::Plan(Plan::DamagedInventory(error)) => {
                repair(&format!("managed tools are damaged or unsafe ({error})"))
            }
            Self::Untrusted => repair(&"managed tools have no matching trusted setup record"),
            Self::Changed => repair(&"managed tools changed after this run selected them"),
            Self::Plan(Plan::InvalidLock) => invalid(&"its tool metadata does not apply here"),
            Self::Plan(Plan::Selection(error)) => invalid(error),
            Self::Plan(Plan::Sources(Storage::Lock(error)))
            | Self::Sources(Storage::Lock(error)) => invalid(error),
            Self::Plan(Plan::Sources(Storage::Declarations))
            | Self::Sources(Storage::Declarations) => {
                "tool declarations are invalid; fix them, then run turbo setup".into()
            }
            Self::Plan(Plan::Sources(Storage::Conflict)) | Self::Sources(Storage::Conflict) => {
                "turbo.lock or tool declarations changed during this run; rerun it, after turbo \
                 setup if declarations changed"
                    .into()
            }
            other => other.to_string(),
        }
    }

    /// `turbo run` guidance for a `turbo setup --no-lock` clone, local or CI:
    /// there is no turbo.lock to update or commit, so every repair is the
    /// same clone-local setup.
    pub(crate) fn no_lock_guidance(&self) -> String {
        use turborepo_setup::{activation::Error as Plan, lock::StorageError as Storage};
        let cause = match self {
            Self::Plan(Plan::DeclarationDrift) => "tool declarations no longer match this clone's \
                                                   `turbo setup --no-lock` selection"
                .into(),
            Self::Plan(Plan::MissingInventory) => "managed tools are not installed".into(),
            Self::Plan(Plan::StaleInventory) => {
                "managed tools do not match the clone-local selection".into()
            }
            Self::Plan(Plan::DamagedInventory(error)) => {
                format!("managed tools are damaged or unsafe ({error})")
            }
            Self::Plan(Plan::InvalidSelection) => "the clone-local selection is invalid".into(),
            Self::Untrusted => "managed tools have no matching trusted setup record".into(),
            Self::Changed => "managed tools changed after this run selected them".into(),
            Self::Plan(Plan::Sources(Storage::Conflict)) | Self::Sources(Storage::Conflict) => {
                return "turbo.lock or tool declarations changed during this run; rerun it, after \
                        turbo setup if a turbo.lock was added"
                    .into();
            }
            Self::Plan(Plan::Sources(Storage::Declarations))
            | Self::Sources(Storage::Declarations) => {
                return "tool declarations are invalid; fix them, then run turbo setup --no-lock"
                    .into();
            }
            other => return other.to_string(),
        };
        format!("{cause}; run turbo setup --no-lock to update this clone's managed tools")
    }
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
    ecosystems: Ecosystems,
    plan: ActivationPlan,
    trust: Option<TrustStore>,
    clone_local: bool,
}

#[allow(dead_code)]
impl Authorization {
    /// User-state base must come from the caller's user directory, never config
    /// authored by the repository. Missing state fails closed without creating
    /// it. `ecosystems` must be the ones setup captured with (root workspace
    /// flags and tools policy), or Rust, Python and Go selections drift.
    pub(crate) fn inspect(
        root: &Path,
        user_state_base: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
    ) -> Result<Self, Error> {
        Self::authorize(root, user_state_base, context, ecosystems, false)
    }

    /// `inspect` for a `turbo setup --no-lock` clone: the sealed clone-local
    /// selection in place of turbo.lock, authorized only by a lockless record
    /// for that selection. Node/npm/pnpm ecosystems only, like `--no-lock`
    /// setup.
    pub(crate) fn inspect_clone_local(
        root: &Path,
        user_state_base: &Path,
        context: ExecutionContext,
    ) -> Result<Self, Error> {
        Self::authorize(root, user_state_base, context, Ecosystems::default(), true)
    }

    fn authorize(
        root: &Path,
        user_state_base: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
        clone_local: bool,
    ) -> Result<Self, Error> {
        let root = root.canonicalize()?;
        let sources = Snapshot::capture_with(&root, ecosystems)?;
        // Only installed tools need setup's trusted record. verify-system tools
        // activate nothing from the repository and keep no user state. A
        // `--no-lock` clone always selects installed tools.
        let installed = clone_local
            || sources
                .previous_lock()
                .is_some_and(|lock| !lock.verify_system_only());
        // musl hosts never install tools: say so before reading user state.
        if installed && context.musl() {
            return Err(turborepo_setup::activation::Error::MuslHost.into());
        }
        let trust = installed
            .then(|| TrustStore::new(user_state_base, &root))
            .transpose()?;
        if let Some(trust) = &trust {
            trust.preflight()?;
        }
        // Initial authorization never trusts repo-writable publication stamps.
        let plan = if clone_local {
            ActivationPlan::verify_clone_local_captured(&sources, context.clone())?
        } else {
            ActivationPlan::verify_captured(&sources, context.clone())?
        };
        if !authorized(trust.as_ref(), &plan)? {
            return Err(Error::Untrusted);
        }
        sources.ensure_current()?;
        Ok(Self {
            root,
            sources,
            context,
            ecosystems,
            plan,
            trust,
            clone_local,
        })
    }

    fn plan(
        root: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
        clone_local: bool,
    ) -> Result<ActivationPlan, Error> {
        Ok(if clone_local {
            ActivationPlan::inspect_clone_local(root, context)?
        } else {
            ActivationPlan::inspect_with(root, context, ecosystems)?
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
        let current = Self::plan(
            &self.root,
            self.context.clone(),
            self.ecosystems,
            self.clone_local,
        )?;
        if current.path_prepend() != self.plan.path_prepend()
            || current.tools() != self.plan.tools()
            || current.snapshot().canonical_identity() != self.plan.snapshot().canonical_identity()
        {
            return Err(Error::Changed);
        }
        if !authorized(self.trust.as_ref(), &current)? {
            return Err(Error::Untrusted);
        }
        self.sources.ensure_current()?;
        Ok(use_plan(&current))
    }
}

/// Without a trust store, only a plan that installs nothing is authorized.
fn authorized(trust: Option<&TrustStore>, plan: &ActivationPlan) -> std::io::Result<bool> {
    match trust {
        Some(trust) => trust.authorizes(plan),
        None => Ok(plan.tools().is_empty()),
    }
}
