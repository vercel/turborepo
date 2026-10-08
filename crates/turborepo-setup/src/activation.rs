//! Read-only Node/pnpm environment planning, not execution authorization.
//! Repo-writable inventory proves readiness only. Consumers must separately
//! authorize setup-recorded tools with trusted-root user state, gate managed
//! mode, and revalidate before use. No CLI support is advertised by this
//! module.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use turborepo_tool_install::{Store, Tool};

use crate::{
    execution_identity::{ExecutionContext, ExecutionSnapshot},
    lock::{Installation, Platform, Snapshot, StorageError},
    node_provision::NodePlan,
    pnpm_provision::PnpmPlan,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("managed activation requires turbo.lock; run turbo setup locally and commit the lock")]
    MissingLock,
    #[error(
        "native declarations disagree with turbo.lock; run turbo setup locally and commit the lock"
    )]
    DeclarationDrift,
    #[error("activation planning supports only managed Node/pnpm; use a supported setup adapter")]
    UnsupportedAdapter,
    #[error(
        "managed activation is unqualified for this platform; use a supported macOS/GNU Linux \
         target"
    )]
    UnsupportedPlatform,
    #[error(
        "invalid locked adapter metadata; run turbo setup locally and commit the repaired lock"
    )]
    InvalidLock,
    #[error("managed installation is missing; run turbo setup")]
    MissingInventory,
    #[error("managed installation does not match turbo.lock; run turbo setup")]
    StaleInventory,
    #[error("managed installation is damaged or unsafe; run turbo setup: {0}")]
    DamagedInventory(#[source] turborepo_tool_install::Error),
    #[error("cannot capture or revalidate setup sources; run turbo setup: {0}")]
    Sources(#[from] StorageError),
    #[error("cannot select the locked execution snapshot; run turbo setup: {0}")]
    Selection(#[from] crate::execution_identity::Error),
    #[error("cannot compose managed PATH with the caller PATH: {0}")]
    Path(#[from] std::env::JoinPathsError),
}

/// An owned selection bound to a checked repo-local generation. This is NOT a
/// ready-to-execute capability: trust/authorization and actual process wiring
/// belong to the consumer. Planning never reads or mutates process environment.
pub struct ActivationPlan {
    snapshot: ExecutionSnapshot,
    bin: PathBuf,
    tools: Vec<Tool>,
    // These adapters require no additional runtime variables. Keep that explicit
    // rather than inheriting or inventing host-dependent runtime settings.
    runtime_env: BTreeMap<String, OsString>,
}

impl ActivationPlan {
    /// Caller selects the exact root and execution context; no root/host
    /// probes, ancestor search, system-tool fallback, resolution, or
    /// storage creation. A missing lock is an error, not unconfigured-mode
    /// policy for future callers.
    pub fn inspect(repo: &Path, context: ExecutionContext) -> Result<Self, Error> {
        let platform = context.artifact_platform();
        if !cfg!(unix)
            || !matches!(
                platform,
                Platform::MacosX64
                    | Platform::MacosArm64
                    | Platform::LinuxX64Gnu
                    | Platform::LinuxArm64Gnu
            )
        {
            return Err(Error::UnsupportedPlatform);
        }
        let repo = repo.canonicalize().map_err(StorageError::from)?;
        let sources = Snapshot::capture(&repo)?;
        Self::from_sources(&repo, &sources, context)
    }

    fn from_sources(
        repo: &Path,
        sources: &Snapshot,
        context: ExecutionContext,
    ) -> Result<Self, Error> {
        let lock = sources.previous_lock().ok_or(Error::MissingLock)?;
        if lock.tools().is_empty()
            || lock.tools().iter().any(|(id, tool)| {
                id != &tool.adapter
                    || !matches!(id.as_str(), "node" | "pnpm")
                    || !matches!(tool.installation, Installation::Managed { .. })
            })
        {
            return Err(Error::UnsupportedAdapter);
        }
        sources
            .check_native_coverage(None)
            .map_err(|_| Error::InvalidLock)?;
        if !lock
            .matches_native(sources.declarations())
            .map_err(|_| Error::InvalidLock)?
        {
            return Err(Error::DeclarationDrift);
        }
        let platform = context.artifact_platform();
        let node = NodePlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
        let mut tools = vec![node.inventory_tool().clone()];
        if lock.tools().contains_key("pnpm") {
            let manager = sources.package_manager()?.ok_or(Error::DeclarationDrift)?;
            // Share setup's full authored-pin validation and reuse identity;
            // an authoritative version does not erase devEngines integrity.
            let pnpm = PnpmPlan::from_declaration(lock, platform, &node, &manager)
                .map_err(|_| Error::InvalidLock)?;
            tools.push(pnpm.inventory_tool().clone());
        }
        let selected: BTreeSet<_> = lock.tools().keys().cloned().collect();
        let snapshot = ExecutionSnapshot::select(lock, context, &selected)?;
        let current = Store::inspect(repo)
            .map_err(Error::DamagedInventory)?
            .ok_or(Error::MissingInventory)?;
        // Complete set equality prevents unrelated exports sharing our PATH.
        if current.tools != tools {
            return Err(Error::StaleInventory);
        }
        sources.ensure_current()?;
        Ok(Self {
            snapshot,
            bin: current.bin,
            tools,
            runtime_env: BTreeMap::new(),
        })
    }

    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }

    /// Validated installation bookkeeping only, never trusted-root evidence.
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn path_prepend(&self) -> &Path {
        &self.bin
    }

    pub fn runtime_env(&self) -> &BTreeMap<String, OsString> {
        &self.runtime_env
    }

    /// Pure composition; the caller supplies PATH and decides whether/how to
    /// apply it after separate authorization. Preserve the original suffix.
    pub fn path(&self, inherited: Option<&OsStr>) -> Result<OsString, Error> {
        let mut paths = vec![self.bin.clone()];
        if let Some(inherited) = inherited {
            paths.extend(std::env::split_paths(inherited));
        }
        Ok(std::env::join_paths(paths)?)
    }
}

#[cfg(all(test, unix))]
mod tests;
