//! Read-only managed Node/npm/pnpm/Rust environment planning, not execution
//! authorization. Repo-writable inventory proves readiness only. Consumers
//! must separately authorize setup-recorded tools with trusted-root user
//! state, gate managed mode, and revalidate before use. No CLI support is
//! advertised by this module.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
};

use turborepo_tool_install::{Store, Tool, Verification};

use crate::{
    execution_identity::{ExecutionContext, ExecutionSnapshot},
    lock::{Ecosystems, Installation, Platform, Snapshot, StorageError},
    node_provision::NodePlan,
    npm_provision::NpmPlan,
    pnpm_provision::PnpmPlan,
    rust_provision::{self, RustPlan},
    rustup_provision::RustupPlan,
};

const SUPPORTED: [&str; 5] = ["node", "npm", "pnpm", "rust", "rustup"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("managed activation requires turbo.lock; run turbo setup locally and commit the lock")]
    MissingLock,
    #[error(
        "native declarations disagree with turbo.lock; run turbo setup locally and commit the lock"
    )]
    DeclarationDrift,
    #[error(
        "activation planning supports only managed Node, npm, pnpm and Rust; use a supported \
         setup adapter"
    )]
    UnsupportedAdapter,
    /// Intentional external tools, unlike a missing managed installation.
    #[error(
        "turbo.lock selects verify-system tools, which come from PATH rather than a managed \
         installation; run turbo setup --check to verify them"
    )]
    VerifySystem,
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
    #[error("unsafe private CARGO_HOME; remove {0} and rerun turbo setup")]
    UnsafeCargoHome(PathBuf),
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
    // Only managed Rust adds variables: its installed private RUSTUP_HOME,
    // exact RUSTUP_TOOLCHAIN and the clone's CARGO_HOME. Never inherited or
    // host-dependent settings.
    runtime_env: BTreeMap<String, OsString>,
}

impl ActivationPlan {
    /// Caller selects the exact root and execution context; no root/host
    /// probes, ancestor search, system-tool fallback, resolution, or
    /// storage creation. A missing lock is an error, not unconfigured-mode
    /// policy for future callers. Rust declarations are ignored, so a lock
    /// with Rust selections is drift; see `inspect_with`.
    pub fn inspect(repo: &Path, context: ExecutionContext) -> Result<Self, Error> {
        Self::inspect_with(repo, context, Ecosystems::default())
    }

    /// Same, with the caller's workspace-flag eligibility, exactly as setup
    /// captured declarations for this repository. Both trust installed bytes
    /// while their publication stamps match (`Verification::Stamped`), for
    /// repeated run-time validation.
    pub fn inspect_with(
        repo: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
    ) -> Result<Self, Error> {
        Self::plan(repo, context, ecosystems, Verification::Stamped)
    }

    /// `inspect_with`, re-hashing every installed file, as `turbo setup
    /// --check` does.
    pub fn verify_with(
        repo: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
    ) -> Result<Self, Error> {
        Self::plan(repo, context, ecosystems, Verification::Full)
    }

    /// Inspect the caller's original source/lock snapshot, never recapture it
    /// after drift. The snapshot binds its own canonical repository root.
    pub fn inspect_captured(sources: &Snapshot, context: ExecutionContext) -> Result<Self, Error> {
        Self::qualify(&context)?;
        sources.ensure_current()?;
        Self::from_sources(
            sources.repository_root(),
            sources,
            context,
            Verification::Stamped,
        )
    }

    fn plan(
        repo: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
        verification: Verification,
    ) -> Result<Self, Error> {
        Self::qualify(&context)?;
        let sources = Snapshot::capture_with(repo, ecosystems)?;
        Self::from_sources(sources.repository_root(), &sources, context, verification)
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
        verification: Verification,
    ) -> Result<Self, Error> {
        let lock = sources.previous_lock().ok_or(Error::MissingLock)?;
        if !lock.tools().is_empty()
            && lock
                .tools()
                .values()
                .all(|tool| matches!(tool.installation, Installation::VerifySystem { .. }))
        {
            return Err(Error::VerifySystem);
        }
        if lock.tools().is_empty()
            || lock.tools().iter().any(|(id, tool)| {
                id != &tool.adapter
                    || !SUPPORTED.contains(&id.as_str())
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
        let mut tools = Vec::new();
        let node = if lock.tools().contains_key("node") || lock.tools().contains_key("pnpm") {
            let node = NodePlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            let version = semver::Version::parse(&node.inventory_tool().version)
                .map_err(|_| Error::InvalidLock)?;
            if !sources
                .node_requirements()?
                .matches_locked_version(&version)
            {
                return Err(Error::DeclarationDrift);
            }
            tools.push(node.inventory_tool().clone());
            Some(node)
        } else {
            None
        };
        if let Some(node) = &node
            && lock.tools().contains_key("pnpm")
        {
            let manager = sources.package_manager()?.ok_or(Error::DeclarationDrift)?;
            // Share setup's full authored-pin validation and reuse identity;
            // an authoritative version does not erase devEngines integrity.
            let pnpm = PnpmPlan::from_declaration(lock, platform, node, &manager)
                .map_err(|_| Error::InvalidLock)?;
            tools.push(pnpm.inventory_tool().clone());
        }
        if lock.tools().contains_key("npm") {
            let manager = sources.package_manager()?.ok_or(Error::DeclarationDrift)?;
            if let Some(npm) = NpmPlan::from_declaration(lock, platform, &node, &manager)
                .map_err(|_| Error::InvalidLock)?
            {
                tools.push(npm.inventory_tool().clone());
            }
        }
        let rust = match lock.tools().get("rust") {
            None => None,
            Some(_) if !lock.tools().contains_key("rustup") => return Err(Error::InvalidLock),
            Some(tool) => {
                // Same semantic binding as setup: the exact channel and options
                // must still satisfy the captured root toolchain file.
                let toolchain = sources.rust_toolchain()?.ok_or(Error::DeclarationDrift)?;
                if !rust_provision::satisfies(&toolchain, tool) {
                    return Err(Error::DeclarationDrift);
                }
                let rust = RustPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
                tools.push(rust.inventory_tool().clone());
                Some(rust)
            }
        };
        if lock.tools().contains_key("rustup") {
            let rustup = RustupPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            tools.push(rustup.inventory_tool().clone());
        }
        // Inventory records its cohort in ID order.
        tools.sort_by(|a, b| a.id.cmp(&b.id));
        let selected: BTreeSet<_> = lock.tools().keys().cloned().collect();
        let snapshot = ExecutionSnapshot::select(lock, context, &selected)?;
        let current = Store::inspect_with(repo, verification)
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
        let mut runtime_env = BTreeMap::new();
        if let Some(rust) = &rust {
            // Shims run the toolchain binaries directly. These variables keep
            // a task's rustup and Cargo on this private, installed toolchain
            // and per-clone Cargo home, never the user's homes or overrides.
            let home = current
                .tool_tree(rust.inventory_tool())
                .ok_or(Error::StaleInventory)?;
            let cargo_home = crate::cargo_prefetch::cargo_home_path(repo);
            // Cargo creates a missing home on first use; never create it here.
            match std::fs::symlink_metadata(&cargo_home) {
                Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                    return Err(Error::UnsafeCargoHome(cargo_home));
                }
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    return Err(StorageError::from(e).into());
                }
                _ => {}
            }
            runtime_env.insert("RUSTUP_HOME".into(), home.into_os_string());
            runtime_env.insert("RUSTUP_TOOLCHAIN".into(), rust.toolchain().into());
            runtime_env.insert("CARGO_HOME".into(), cargo_home.into_os_string());
        }
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

    /// Absolute values to set for tasks and turbo's own tool probes, after
    /// any strict/loose filtering. They contain local paths: never hash them.
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
