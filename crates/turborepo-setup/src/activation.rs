//! Read-only Node/npm/pnpm/Yarn/Bun/Rust/Go environment planning, plus
//! uv/Python when enabled, and repository-declared additional tools, not
//! execution authorization.
//! Repo-writable inventory proves readiness only. Consumers must separately
//! authorize setup-recorded tools with trusted-root user state, gate managed
//! mode, and revalidate before use. No CLI support is advertised by this
//! module.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
};

use turborepo_tool_install::{Record, Store, Tool, Verification};

use crate::{
    artifact_provision::{self, ArtifactPlan},
    bun_provision::BunPlan,
    execution_identity::{ExecutionContext, ExecutionSnapshot},
    go_discovery::GoVersion,
    go_provision::GoPlan,
    lock::{Ecosystems, Installation, Lock, Platform, Snapshot, StorageError, additional},
    node_provision::NodePlan,
    npm_provision::NpmPlan,
    package_manager::{Declaration, Manager},
    pnpm_provision::PnpmPlan,
    python_provision::PythonPlan,
    rust_provision::{self, RustPlan},
    rustup_provision::RustupPlan,
    uv_provision::UvPlan,
    yarn_provision::YarnPlan,
};

const SUPPORTED: [&str; 6] = ["node", "npm", "pnpm", "rust", "rustup", "go"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("managed activation requires turbo.lock; run turbo setup locally and commit the lock")]
    MissingLock,
    #[error(
        "native declarations disagree with turbo.lock; run turbo setup locally and commit the lock"
    )]
    DeclarationDrift,
    #[error(
        "activation planning supports managed Node, npm, pnpm, Yarn, Bun, Rust, Go and additional \
         tools, plus uv and Python with experimentalPythonWorkspaces; use a supported setup \
         adapter"
    )]
    UnsupportedAdapter,
    /// A locked additional tool without an artifact for this platform, or
    /// whose export would shadow another selected tool.
    #[error(transparent)]
    AdditionalTool(artifact_provision::Error),
    #[error(
        "managed activation is unqualified for this platform; use a supported macOS, GNU Linux or \
         Windows x64 target"
    )]
    UnsupportedPlatform,
    #[error(
        "managed tools are not supported on musl Linux hosts such as Alpine; install the managed \
         tools on a supported macOS, GNU Linux or Windows x64 host"
    )]
    MuslHost,
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
    #[error("invalid clone-local setup selection; run turbo setup --no-lock")]
    InvalidSelection,
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
    // Rust, Go and uv/Python settings derived from this generation alone; see
    // [`runtime_env`].
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

    /// Use setup's exact workspace-flag eligibility when capturing
    /// declarations.
    /// Repeated inspection uses publication stamps; explicit verification
    /// hashes bytes.
    pub fn inspect_with(
        repo: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
    ) -> Result<Self, Error> {
        Self::qualify(&context)?;
        let sources = Snapshot::capture_with(repo, ecosystems)?;
        Self::inspect_captured(&sources, context)
    }

    /// Inspect the original source snapshot with publication-stamp caching.
    pub fn inspect_captured(sources: &Snapshot, context: ExecutionContext) -> Result<Self, Error> {
        Self::captured(sources, context, Verification::Stamped)
    }

    /// Re-hash all installed bytes while retaining the caller's original
    /// sources.
    pub fn verify_captured(sources: &Snapshot, context: ExecutionContext) -> Result<Self, Error> {
        Self::captured(sources, context, Verification::Full)
    }

    /// Full verification for callers selecting declaration ecosystems
    /// explicitly.
    pub fn verify_with(
        repo: &Path,
        context: ExecutionContext,
        ecosystems: Ecosystems,
    ) -> Result<Self, Error> {
        Self::qualify(&context)?;
        let sources = Snapshot::capture_with(repo, ecosystems)?;
        Self::verify_captured(&sources, context)
    }

    /// A `turbo setup --no-lock` clone: plan the sealed clone-local selection
    /// its generation records, in place of turbo.lock, with the same
    /// declaration, adapter and installation checks. A turbo.lock in the root
    /// is the committed resolution and is never overridden. Node/npm/pnpm
    /// ecosystems only, like `--no-lock` setup. The snapshot identity is
    /// clone-local (see `ExecutionSnapshot::select_clone_local`).
    pub fn inspect_clone_local(repo: &Path, context: ExecutionContext) -> Result<Self, Error> {
        Self::qualify(&context)?;
        // A clone-local selection is always an installed generation.
        if context.musl() {
            return Err(Error::MuslHost);
        }
        let sources = Snapshot::capture(repo)?;
        Self::clone_local(&sources, context, Verification::Stamped)
    }

    /// `inspect_clone_local` over the caller's original sources, re-hashing
    /// all installed bytes.
    pub fn verify_clone_local_captured(
        sources: &Snapshot,
        context: ExecutionContext,
    ) -> Result<Self, Error> {
        Self::clone_local(sources, context, Verification::Full)
    }

    fn clone_local(
        sources: &Snapshot,
        context: ExecutionContext,
        verification: Verification,
    ) -> Result<Self, Error> {
        Self::qualify(&context)?;
        sources.ensure_current()?;
        if sources.previous_lock().is_some() {
            return Err(StorageError::Conflict.into());
        }
        let current = Store::inspect(sources.repository_root())
            .map_err(Error::DamagedInventory)?
            .ok_or(Error::MissingInventory)?;
        let record = current.record.ok_or(Error::StaleInventory)?;
        let native = crate::native_baseline::NativeRecord::decode(record.bytes())
            .map_err(|_| Error::InvalidSelection)?;
        Self::from_selection(
            sources.repository_root(),
            sources,
            native.selection(),
            Some(&record),
            context,
            verification,
        )
    }

    fn captured(
        sources: &Snapshot,
        context: ExecutionContext,
        verification: Verification,
    ) -> Result<Self, Error> {
        Self::qualify(&context)?;
        sources.ensure_current()?;
        Self::from_sources(sources.repository_root(), sources, context, verification)
    }

    fn qualify(context: &ExecutionContext) -> Result<(), Error> {
        let qualified = match context.artifact_platform() {
            Platform::MacosX64
            | Platform::MacosArm64
            | Platform::LinuxX64Gnu
            | Platform::LinuxArm64Gnu
            | Platform::LinuxX64Musl
            | Platform::LinuxArm64Musl => cfg!(unix),
            Platform::WindowsX64 => cfg!(windows),
            _ => false,
        };
        if !qualified {
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
        Self::from_selection(repo, sources, lock, None, context, verification)
    }

    /// `lock` is turbo.lock, or with `recorded`, the clone-local selection
    /// decoded from that exact generation record.
    fn from_selection(
        repo: &Path,
        sources: &Snapshot,
        lock: &Lock,
        recorded: Option<&Record>,
        context: ExecutionContext,
        verification: Verification,
    ) -> Result<Self, Error> {
        // Before adapter checks, so musl users get the actionable reason.
        if context.musl() {
            return Err(Error::MuslHost);
        }
        let python = sources.ecosystems().python;
        if lock.tools().is_empty()
            || lock.tools().iter().any(|(id, tool)| {
                let builtin = id == &tool.adapter
                    && (SUPPORTED.contains(&id.as_str())
                        || matches!(id.as_str(), "yarn" | "bun")
                        || (python && matches!(id.as_str(), "uv" | "python")));
                let installation = match &tool.installation {
                    Installation::Managed { .. } => true,
                    Installation::Bundled { .. } => id == "npm",
                };
                !(builtin || tool.adapter == additional::ADAPTER) || !installation
            })
        {
            return Err(Error::UnsupportedAdapter);
        }
        // Without the Go workspace flag its declarations are unread, so a
        // locked Go selection is drift, as in setup, never activated.
        if !sources.ecosystems().go && lock.tools().contains_key("go") {
            return Err(Error::DeclarationDrift);
        }
        sources
            .check_native_coverage(Some(lock))
            .map_err(|_| Error::InvalidLock)?;
        // Additional-tool entries are projections of the root configuration,
        // so any difference, including a removed declaration, is drift.
        if !lock
            .matches_native(sources.declarations())
            .map_err(|_| Error::InvalidLock)?
            || !sources.matches_additional(lock)
        {
            return Err(Error::DeclarationDrift);
        }
        let platform = context.artifact_platform();
        let version =
            |tool: &Tool| semver::Version::parse(&tool.version).map_err(|_| Error::InvalidLock);
        let mut tools = Vec::new();
        // Python- and Go-only repositories lock no Node; pnpm still requires it.
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
        if ["npm", "pnpm", "yarn", "bun"]
            .iter()
            .any(|id| lock.tools().contains_key(*id))
        {
            let manager = sources.package_manager()?.ok_or(Error::DeclarationDrift)?;
            if let Some(tool) = manager_tool(lock, platform, node.as_ref(), &manager)? {
                tools.push(tool);
            }
        }
        // Inventory records tools in ID order.
        if lock.tools().contains_key("uv") {
            let plan = UvPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            let requirement = sources.uv_requirement()?.ok_or(Error::DeclarationDrift)?;
            if !requirement.matches(&version(plan.inventory_tool())?) {
                return Err(Error::DeclarationDrift);
            }
            tools.push(plan.inventory_tool().clone());
        }
        if lock.tools().contains_key("python") {
            let plan = PythonPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            let requirement = sources
                .python_requirement()?
                .ok_or(Error::DeclarationDrift)?;
            if !requirement.matches(&version(plan.inventory_tool())?) {
                return Err(Error::DeclarationDrift);
            }
            tools.push(plan.inventory_tool().clone());
        }
        // The Store records a generation's tools in id order.
        if let Some(tool) = lock.tools().get("rust") {
            if !lock.tools().contains_key("rustup") {
                return Err(Error::InvalidLock);
            }
            // Same semantic binding as setup: the exact channel and options
            // must still satisfy the captured root toolchain file.
            let toolchain = sources.rust_toolchain()?.ok_or(Error::DeclarationDrift)?;
            if !rust_provision::satisfies(&toolchain, tool) {
                return Err(Error::DeclarationDrift);
            }
            let rust = RustPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            tools.push(rust.inventory_tool().clone());
        }
        if lock.tools().contains_key("rustup") {
            let rustup = RustupPlan::from_lock(lock, platform).map_err(|_| Error::InvalidLock)?;
            tools.push(rustup.inventory_tool().clone());
        }
        let go = (lock.tools().contains_key("go"))
            .then(|| GoPlan::from_lock(lock, platform))
            .transpose()
            .map_err(|_| Error::InvalidLock)?;
        if let Some(go) = &go {
            let version =
                GoVersion::parse(&go.inventory_tool().version).ok_or(Error::InvalidLock)?;
            // Matching provenance cannot authorize a release outside the native
            // minimum or an exact `toolchain` preference.
            if !sources
                .go_requirements()?
                .is_some_and(|requirements| requirements.matches_locked(version))
            {
                return Err(Error::DeclarationDrift);
            }
            tools.push(go.inventory_tool().clone());
        }
        // Declared executables share the managed bin directory with the
        // built-in exports; their paths stay relative to the full installed
        // tree, so executable-relative resources keep resolving.
        let artifacts = ArtifactPlan::from_lock(lock, platform).map_err(|error| match error {
            artifact_provision::Error::MissingTarget { .. } => Error::AdditionalTool(error),
            _ => Error::InvalidLock,
        })?;
        let declared = artifacts.iter().map(ArtifactPlan::inventory_tool);
        artifact_provision::check_exports(&tools, declared.clone())
            .map_err(Error::AdditionalTool)?;
        tools.extend(declared.cloned());
        // Inventory records its cohort in ID order.
        tools.sort_by(|a, b| a.id.cmp(&b.id));
        let selected: BTreeSet<_> = lock.tools().keys().cloned().collect();
        let snapshot = match recorded {
            None => ExecutionSnapshot::select(lock, context, &selected)?,
            Some(_) => ExecutionSnapshot::select_clone_local(lock, context, &selected)?,
        };
        let current = Store::inspect_with(repo, verification)
            .map_err(Error::DamagedInventory)?
            .ok_or(Error::MissingInventory)?;
        // Complete set equality prevents unrelated exports sharing our PATH.
        // A clone-local plan also stays bound to the record it decoded.
        if current.tools != tools || recorded.is_some_and(|r| current.record.as_ref() != Some(r)) {
            return Err(Error::StaleInventory);
        }
        if let Some(node) = &node {
            let tree = current
                .tool_tree(node.inventory_tool())
                .ok_or(Error::StaleInventory)?;
            node.verify_bundled_npm(&tree)
                .map_err(|_| Error::StaleInventory)?;
        }
        // The shell hook derives the same values from its trusted record.
        let runtime_env = runtime_env(repo, &current.bin, &tools)?;
        sources.ensure_current()?;
        Ok(Self {
            snapshot,
            // PATH consumers such as cmd.exe do not resolve `%~dp0..` under a
            // verbatim `\\?\` prefix; use the ordinary spelling when exact.
            #[cfg(windows)]
            bin: dunce::simplified(&current.bin).to_path_buf(),
            #[cfg(not(windows))]
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

    /// The verified managed bin directory.
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

/// Every variable [`runtime_env`] may set, sorted. The shell hook restores only
/// these from its saved state, so a tampered state cannot unset or overwrite
/// any other variable.
pub const HOOK_ENV: [&str; 10] = [
    "CARGO_HOME",
    "GOROOT",
    "GOTOOLCHAIN",
    "RUSTUP_AUTO_INSTALL",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "UV_PYTHON",
    "UV_PYTHON_DOWNLOADS",
    "UV_PYTHON_INSTALL_DIR",
    "UV_PYTHON_PREFERENCE",
];

/// The variables an installed generation's tools need beside PATH, from its
/// `bin` and inventory tools alone. Managed run and exec set exactly these,
/// and the shell hook derives them from its trusted record without reading the
/// Store, so the prompt and tasks always agree. Never inherited or
/// host-dependent settings:
///
/// - Rust: the generation's private `RUSTUP_HOME`, the exact `RUSTUP_TOOLCHAIN`
///   and the clone's `CARGO_HOME`, never the user's homes or overrides. Windows
///   exports are rustup proxies, which must never auto-install into the
///   generation.
/// - Go: `GOTOOLCHAIN=local` stops toolchain switches and downloads, and
///   `GOROOT` pins this verified tree, so the locked `go` never builds with
///   another release's standard library and tools. Windows `go.exe` is a hard
///   link that finds its tree only through `GOROOT`.
/// - uv/Python: the uv launchers' settings, for uv processes that do not start
///   through them (every Windows uv), plus the locked interpreter as uv's
///   request. That is uv's download key rather than a path: a path request
///   makes `uv pip` install into the managed interpreter itself instead of the
///   project's virtual environment.
///
/// Node and Bun are self-contained, and pnpm/Yarn shims launch the selected
/// managed Node by path, so they need none.
pub fn runtime_env(
    root: &Path,
    bin: &Path,
    tools: &[Tool],
) -> Result<BTreeMap<String, OsString>, Error> {
    let mut env = BTreeMap::new();
    let tool = |id: &str| tools.iter().find(|tool| tool.id == id);
    let (rust, go, uv, python) = (tool("rust"), tool("go"), tool("uv"), tool("python"));
    if rust.is_none() && go.is_none() && uv.is_none() && python.is_none() {
        return Ok(env);
    }
    let generation = bin.parent().ok_or(Error::StaleInventory)?;
    // Like `bin`, the ordinary spelling tools print and compare, never a
    // verbatim `\\?\` path.
    let plain = |path: PathBuf| {
        #[cfg(windows)]
        let path = dunce::simplified(&path).to_path_buf();
        path.into_os_string()
    };
    let tree = |id: &str| plain(generation.join("tools").join(id));
    if let Some(rust) = rust {
        let host = crate::rustup::host(&rust.platform).ok_or(Error::InvalidLock)?;
        let cargo_home = crate::cargo_prefetch::cargo_home_path(root);
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
        if cfg!(windows) {
            env.insert("RUSTUP_AUTO_INSTALL".into(), "0".into());
        }
        env.insert("RUSTUP_HOME".into(), tree("rust"));
        let toolchain = format!("{}-{host}", rust.version);
        env.insert("RUSTUP_TOOLCHAIN".into(), toolchain.into());
        env.insert("CARGO_HOME".into(), plain(cargo_home));
    }
    if go.is_some() {
        for (key, value) in crate::go_provision::RUNTIME_ENV {
            env.insert(key.to_owned(), OsString::from(value));
        }
        env.insert("GOROOT".into(), tree("go"));
    }
    if uv.is_some() || python.is_some() {
        env.insert("UV_PYTHON_DOWNLOADS".into(), "never".into());
        env.insert("UV_PYTHON_INSTALL_DIR".into(), tree("python"));
    }
    if let Some(python) = python {
        let key = crate::python_resolution::installed_key(&python.version, &python.platform)
            .ok_or(Error::InvalidLock)?;
        env.insert("UV_PYTHON_PREFERENCE".into(), "only-managed".into());
        env.insert("UV_PYTHON".into(), key.into());
    }
    Ok(env)
}

/// The installed identity setup provisions for the declared manager. Each plan
/// shares setup's full authored-pin validation and reuse identity, so an
/// authoritative version never erases devEngines integrity.
fn manager_tool(
    lock: &Lock,
    platform: Platform,
    node: Option<&NodePlan>,
    manager: &Declaration,
) -> Result<Option<Tool>, Error> {
    let node = || node.ok_or(Error::InvalidLock);
    let id = match manager.manager {
        Manager::Npm => "npm",
        Manager::Pnpm => "pnpm",
        Manager::Yarn => "yarn",
        Manager::Bun => "bun",
        _ => return Err(Error::UnsupportedAdapter),
    };
    if !lock.tools().contains_key(id) {
        return Err(Error::DeclarationDrift);
    }
    if manager.manager == Manager::Npm {
        return NpmPlan::from_declaration(lock, platform, node()?, manager)
            .map(|plan| plan.map(|plan| plan.inventory_tool().clone()))
            .map_err(|_| Error::InvalidLock);
    }
    Ok(Some(match manager.manager {
        Manager::Pnpm => PnpmPlan::from_declaration(lock, platform, node()?, manager)
            .map_err(|_| Error::InvalidLock)?
            .inventory_tool()
            .clone(),
        Manager::Yarn => YarnPlan::from_declaration(lock, platform, node()?, manager)
            .map_err(|_| Error::InvalidLock)?
            .inventory_tool()
            .clone(),
        _ => BunPlan::from_declaration(lock, platform, manager)
            .map_err(|_| Error::InvalidLock)?
            .inventory_tool()
            .clone(),
    }))
}

#[cfg(all(test, unix))]
mod tests;
