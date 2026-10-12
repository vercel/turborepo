//! Read-only Node/npm/pnpm environment planning, plus uv/Python when enabled,
//! not execution authorization.
//! Repo-writable inventory proves readiness only. Consumers must separately
//! authorize setup-recorded tools with trusted-root user state, gate managed
//! mode, and revalidate before use. No CLI support is advertised by this
//! module.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use turborepo_tool_install::{Current, Store, Tool};

use crate::{
    execution_identity::{ExecutionContext, ExecutionSnapshot},
    lock::{Ecosystems, Installation, Platform, Snapshot, StorageError},
    node_provision::NodePlan,
    npm_provision::NpmPlan,
    pnpm_provision::PnpmPlan,
    python_provision::PythonPlan,
    uv_provision::UvPlan,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("managed activation requires turbo.lock; run turbo setup locally and commit the lock")]
    MissingLock,
    #[error(
        "native declarations disagree with turbo.lock; run turbo setup locally and commit the lock"
    )]
    DeclarationDrift,
    #[error(
        "activation planning supports managed Node/npm/pnpm, plus uv and Python with \
         experimentalPythonWorkspaces; use a supported setup adapter"
    )]
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
    // Node/pnpm need no runtime variables. uv/Python values derive only from
    // this generation and the lock, never from host or user settings.
    runtime_env: BTreeMap<String, OsString>,
}

impl ActivationPlan {
    /// Caller selects the exact root and execution context; no root/host
    /// probes, ancestor search, system-tool fallback, resolution, or
    /// storage creation. A missing lock is an error, not unconfigured-mode
    /// policy for future callers.
    pub fn inspect(repo: &Path, context: ExecutionContext) -> Result<Self, Error> {
        Self::qualify(&context)?;
        let sources = Snapshot::capture(repo)?;
        Self::inspect_captured(&sources, context)
    }

    /// Inspect a root with explicitly enabled declaration ecosystems.
    pub fn inspect_with(
        repo: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
    ) -> Result<Self, Error> {
        Self::qualify(&context)?;
        let sources = Snapshot::capture_with(repo, ecosystems)?;
        Self::inspect_captured(&sources, context)
    }

    /// Inspect the caller's original source/lock snapshot, never recapture it
    /// after drift. The snapshot binds its own canonical repository root.
    pub fn inspect_captured(sources: &Snapshot, context: ExecutionContext) -> Result<Self, Error> {
        Self::qualify(&context)?;
        sources.ensure_current()?;
        Self::from_sources(sources.repository_root(), sources, context)
    }

    fn qualify(context: &ExecutionContext) -> Result<(), Error> {
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
        Ok(())
    }

    fn from_sources(
        repo: &Path,
        sources: &Snapshot,
        context: ExecutionContext,
    ) -> Result<Self, Error> {
        let lock = sources.previous_lock().ok_or(Error::MissingLock)?;
        let python = sources.ecosystems().python;
        if lock.tools().is_empty()
            || lock.tools().iter().any(|(id, tool)| {
                id != &tool.adapter
                    || !(matches!(id.as_str(), "node" | "npm" | "pnpm")
                        || (python && matches!(id.as_str(), "uv" | "python")))
                    || !(matches!(tool.installation, Installation::Managed { .. })
                        || (id == "npm"
                            && matches!(tool.installation, Installation::Bundled { .. })))
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
        let version =
            |tool: &Tool| semver::Version::parse(&tool.version).map_err(|_| Error::InvalidLock);
        let mut tools = Vec::new();
        // Python-only repositories lock no Node; pnpm still requires it.
        let node = if lock.tools().contains_key("node") {
            let node = NodePlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            if !sources
                .node_requirements()?
                .matches_locked_version(&version(node.inventory_tool())?)
            {
                return Err(Error::DeclarationDrift);
            }
            tools.push(node.inventory_tool().clone());
            Some(node)
        } else {
            None
        };
        if lock.tools().contains_key("pnpm") {
            let node = node.as_ref().ok_or(Error::InvalidLock)?;
            let manager = sources.package_manager()?.ok_or(Error::DeclarationDrift)?;
            // Share setup's full authored-pin validation and reuse identity;
            // an authoritative version does not erase devEngines integrity.
            let pnpm = PnpmPlan::from_declaration(lock, platform, node, &manager)
                .map_err(|_| Error::InvalidLock)?;
            tools.push(pnpm.inventory_tool().clone());
        }
        if lock.tools().contains_key("npm") {
            let node = node.as_ref().ok_or(Error::InvalidLock)?;
            let manager = sources.package_manager()?.ok_or(Error::DeclarationDrift)?;
            if let Some(npm) = NpmPlan::from_declaration(lock, platform, node, &manager)
                .map_err(|_| Error::InvalidLock)?
            {
                tools.push(npm.inventory_tool().clone());
            }
        }
        let uv = lock.tools().contains_key("uv");
        if uv {
            let plan = UvPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            let requirement = sources.uv_requirement()?.ok_or(Error::DeclarationDrift)?;
            if !requirement.matches(&version(plan.inventory_tool())?) {
                return Err(Error::DeclarationDrift);
            }
            tools.push(plan.inventory_tool().clone());
        }
        let python = if lock.tools().contains_key("python") {
            let plan = PythonPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            let requirement = sources
                .python_requirement()?
                .ok_or(Error::DeclarationDrift)?;
            if !requirement.matches(&version(plan.inventory_tool())?) {
                return Err(Error::DeclarationDrift);
            }
            tools.push(plan.inventory_tool().clone());
            Some(plan)
        } else {
            None
        };
        // The Store records a generation's tools in id order.
        tools.sort_by(|a, b| a.id.cmp(&b.id));
        let selected: BTreeSet<_> = lock.tools().keys().cloned().collect();
        let snapshot = ExecutionSnapshot::select(lock, context, &selected)?;
        let current = Store::inspect(repo)
            .map_err(Error::DamagedInventory)?
            .ok_or(Error::MissingInventory)?;
        // Complete set equality prevents unrelated exports sharing our PATH.
        if current.tools != tools {
            return Err(Error::StaleInventory);
        }
        if let Some(node) = &node {
            let tree = current
                .tool_tree(node.inventory_tool())
                .ok_or(Error::StaleInventory)?;
            node.verify_bundled_npm(&tree)
                .map_err(|_| Error::StaleInventory)?;
        }
        let runtime_env = uv_env(&current, uv, python.as_ref())?;
        sources.ensure_current()?;
        Ok(Self {
            snapshot,
            bin: current.bin,
            tools,
            runtime_env,
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

/// The uv launchers' settings, for uv processes that do not start through
/// them, plus the locked interpreter as uv's request. Uses uv's download key
/// rather than a path: a path request makes `uv pip` install into the managed
/// interpreter itself instead of the project's virtual environment.
fn uv_env(
    current: &Current,
    uv: bool,
    python: Option<&PythonPlan>,
) -> Result<BTreeMap<String, OsString>, Error> {
    let mut env = BTreeMap::new();
    if !uv && python.is_none() {
        return Ok(env);
    }
    let generation = current.bin.parent().ok_or(Error::StaleInventory)?;
    env.insert("UV_PYTHON_DOWNLOADS".into(), "never".into());
    env.insert(
        "UV_PYTHON_INSTALL_DIR".into(),
        generation.join("tools").join("python").into_os_string(),
    );
    if let Some(python) = python {
        env.insert("UV_PYTHON_PREFERENCE".into(), "only-managed".into());
        env.insert("UV_PYTHON".into(), python.key().into());
    }
    Ok(env)
}

#[cfg(all(test, unix))]
mod tests;
