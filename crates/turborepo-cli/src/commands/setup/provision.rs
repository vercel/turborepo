//! Local reconciliation followed by locked tool provisioning.
//! Force reinstalls committed selections without resolution or lock writes.
//! Offline uses only healthy installs and verified cached artifacts.
//! No probes, dependency execution or activation.
use std::{fs, io, path::Path, process::Command};

use turborepo_setup::{
    artifact_cache::ArtifactCache,
    lock::{Installation, Platform, Snapshot},
    mirror::Mirrors,
    node_provision::{NodePlan, NodeTransport},
    npm_provision::NpmPlan,
    package_manager::Manager,
    pnpm_provision::{PnpmPlan, PnpmTransport},
    rust_provision::RustPlan,
    rustup_provision::{RustupPlan, RustupTransport},
    python_provision::PythonPlan,
    python_resolution::PythonTransport,
    source_policy::OfficialSourcePolicy,
    uv_provision::UvPlan,
    uv_resolution::UvTransport,
};
use turborepo_tool_install::Store;

use super::Error;

const SUPPORTED: [&str; 7] = ["node", "npm", "pnpm", "rust", "rustup", "uv", "python"];

pub(super) struct Transports {
    pub node: NodeTransport,
    pub pnpm: PnpmTransport,
    pub registry: turborepo_setup::registry_resolution::RegistryTransport,
    #[cfg(test)]
    pub trust_base: Option<std::path::PathBuf>,
    /// The shared download cache the transports use, cleaned up after setup.
    pub cache: Option<ArtifactCache>,
    pub rustup: RustupTransport,
    pub uv: UvTransport,
    pub python: PythonTransport,
}
impl Transports {
    pub fn official(policy: &OfficialSourcePolicy) -> Result<Self, Error> {
        // Verified archive bytes are shared through the user's cache directory;
        // extraction and activation stay in each repository's .turbo/tools.
        let cache = dirs_next::cache_dir().and_then(|base| ArtifactCache::new(&base));
        Self::configured(policy, &Mirrors::from_env()?, cache)
    }

    pub fn configured(
        policy: &OfficialSourcePolicy,
        mirrors: &Mirrors,
        cache: Option<ArtifactCache>,
    ) -> Result<Self, Error> {
        let registry = || match &mirrors.registry {
            Some(mirror) => PnpmTransport::mirrored(policy, mirror),
            None => PnpmTransport::official(policy),
        };
        let node = match &mirrors.node {
            Some(mirror) => NodeTransport::mirrored(policy, mirror)?,
            None => NodeTransport::official(policy)?,
        };
        Ok(Self {
            node: node.with_cache(cache.clone()),
            pnpm: registry()?.with_cache(cache.clone()),
            registry: registry()?,
            #[cfg(test)]
            trust_base: None,
            cache,
            rustup: RustupTransport::official(policy)?,
            uv: UvTransport::official(policy)?,
            python: PythonTransport::official(policy)?,
        })
    }

    pub(super) fn trust_base(&self) -> Result<std::path::PathBuf, io::Error> {
        #[cfg(test)]
        if let Some(base) = &self.trust_base {
            return Ok(base.clone());
        }
        dirs_next::data_local_dir()
            .ok_or_else(|| io::Error::other("cannot locate setup user state directory"))
    }

    // Keeps each transport's cache; every network path fails before traffic.
    fn offline(self) -> Self {
        Self {
            node: self.node.offline(),
            pnpm: self.pnpm.offline(),
            registry: self.registry.offline(),
            #[cfg(test)]
            trust_base: self.trust_base,
            cache: self.cache,
            rustup: self.rustup,
            uv: self.uv,
            python: self.python,
        }
    }
}

// Lock-derived, credential-free provenance for each missing host artifact.
fn missing_artifacts(
    lock: &turborepo_setup::lock::Lock,
    platform: Platform,
    ids: &[&str],
) -> String {
    let mut lines = Vec::new();
    for (id, tool) in lock
        .tools()
        .iter()
        .filter(|(id, _)| ids.contains(&id.as_str()))
    {
        let Installation::Managed { artifacts } = &tool.installation else {
            continue;
        };
        let parts = artifacts
            .get(&platform)
            .or_else(|| artifacts.get(&Platform::Any));
        for (part, artifact) in parts.into_iter().flatten() {
            lines.push(format!(
                "  {id} {} [{part}]: {} (SHA-256 {})",
                tool.version, artifact.url, artifact.sha256
            ));
        }
    }
    lines.join("\n")
}

pub(super) fn platform() -> Result<Platform, Error> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok(Platform::MacosX64),
        ("macos", "aarch64") => Ok(Platform::MacosArm64),
        ("linux", "x86_64") => glibc_host(Platform::LinuxX64Gnu),
        ("linux", "aarch64") => glibc_host(Platform::LinuxArm64Gnu),
        _ => Err(Error::Unsupported(
            "managed promotion requires macOS or GNU Linux",
        )),
    }
}

/// Released Linux `turbo` binaries are musl-static, so GNU artifacts follow
/// the host's C library rather than this binary's target.
fn glibc_host(platform: Platform) -> Result<Platform, Error> {
    use turborepo_platform::Libc;
    match turborepo_platform::host_libc() {
        Ok(Some(Libc::Glibc { .. })) => Ok(platform),
        Ok(Some(Libc::Musl)) => Err(Error::MuslHost),
        Ok(None) => Err(Error::Unsupported(
            "managed promotion requires macOS or GNU Linux",
        )),
        Err(error) => Err(Error::UnclassifiedLibc(error)),
    }
}

pub(super) fn revalidate(discovery: &super::root::Discovery) -> Result<(), Error> {
    if !discovery.revalidate()? {
        return Err(Error::Unsupported(
            "setup discovery changed during provisioning",
        ));
    }
    Ok(())
}

pub(super) fn storage(root: &Path) -> Result<(), Error> {
    for path in [
        root.join(".turbo"),
        root.join(".turbo/tools"),
        root.join(".turbo/setup-lock"),
    ] {
        match fs::symlink_metadata(path) {
            Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                return Err(Error::Unsupported("unsafe .turbo storage path"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    // Nothing can be committed without a repository, so there is nothing to
    // check. Any `.git` at or above the root keeps the strict checks below.
    if turborepo_setup::writer_storage::outside_git_repository(root)? {
        return Ok(());
    }
    let git = |arguments: &[&str]| {
        Command::new("git")
            .current_dir(root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(arguments)
            .output()
            .map_err(turborepo_setup::writer_storage::git_spawn_error)
    };
    let ignored = git(&["check-ignore", "--", ".turbo/"])?;
    // Exit 128 is a Git failure (for example, a broken or untrusted `.git`), not
    // a missing ignore rule; editing .gitignore would not help.
    if ignored.status.code() == Some(128) {
        return Err(io::Error::other(format!(
            "Git could not check whether .turbo/ is ignored ({}); run turbo setup inside the \
             repository's Git checkout",
            String::from_utf8_lossy(&ignored.stderr).trim()
        ))
        .into());
    }
    let tracked = git(&["ls-files", "--", ".turbo/"])?;
    if !ignored.status.success()
        || ignored.stdout != b".turbo/\n"
        || !tracked.status.success()
        || !tracked.stdout.is_empty()
    {
        return Err(Error::Unsupported(
            "storage must be untracked and Git-ignored; add /.turbo/ to .gitignore",
        ));
    }
    Ok(())
}

/// Read-only current-host selection, shared by frozen execution and checked
/// publication. No storage, transports, probes or writes are initialized here.
pub(super) struct Plans {
    pub node: Option<NodePlan>,
    pub pnpm: Option<PnpmPlan>,
    pub npm: Option<NpmPlan>,
    pub rustup: Option<RustupPlan>,
    pub rust: Option<RustPlan>,
    pub uv: Option<UvPlan>,
    pub python: Option<PythonPlan>,
}

pub(super) fn plans(
    snapshot: &Snapshot,
    lock: &turborepo_setup::lock::Lock,
) -> Result<Plans, Error> {
    let platform = platform()?;
    super::lock_diagnostics::matching(snapshot, lock)?;
    if lock.tools().is_empty()
        || lock
            .tools()
            .keys()
            .any(|id| !SUPPORTED.contains(&id.as_str()))
    {
        return Err(Error::Unsupported(
            "only managed Node, npm, pnpm, Rust, uv, and Python are supported",
        ));
    }
    let node = if lock.tools().contains_key("node") {
        let node = NodePlan::from_lock(lock, platform)?;
        let version = semver::Version::parse(&node.inventory_tool().version)
            .map_err(|_| Error::Unsupported("invalid locked Node version"))?;
        if !snapshot
            .node_requirements()?
            .matches_locked_version(&version)
        {
            return Err(Error::Unsupported(
                "locked Node does not satisfy native declarations",
            ));
        }
        Some(node)
    } else {
        None
    };
    let (mut pnpm, mut npm) = (None, None);
    if let Some(manager) = snapshot.package_manager()? {
        match (manager.manager, &node) {
            (Manager::Pnpm, Some(node)) => {
                pnpm = Some(PnpmPlan::from_declaration(lock, platform, node, &manager)?);
            }
            (Manager::Npm, Some(node)) => {
                npm = NpmPlan::from_declaration(lock, platform, node, &manager)?;
            }
            (Manager::Pnpm | Manager::Npm, None) => {
                return Err(Error::Unsupported(
                    "a managed package manager requires a native Node declaration",
                ));
            }
            _ => return Err(Error::Unsupported("package manager provisioning")),
        }
    }
    let rustup = lock
        .tools()
        .contains_key("rustup")
        .then(|| RustupPlan::from_lock(lock, platform))
        .transpose()?;
    let rust = match (lock.tools().get("rust"), &rustup) {
        (None, _) => None,
        (Some(_), None) => return Err(Error::Unsupported("managed Rust requires locked rustup")),
        (Some(tool), Some(_)) => {
            let toolchain = snapshot.rust_toolchain()?.ok_or(Error::Unsupported(
                "managed Rust requires a root rust-toolchain declaration",
            ))?;
            if !turborepo_setup::rust_provision::satisfies(&toolchain, tool) {
                return Err(Error::Unsupported(
                    "locked Rust does not satisfy the rust-toolchain declaration",
                ));
            }
            Some(RustPlan::from_lock(lock, platform)?)
        }
    };
    let uv = match snapshot.uv_requirement()? {
        Some(requirement) => {
            let plan = UvPlan::from_lock(lock, platform)?;
            let version = semver::Version::parse(&plan.inventory_tool().version)
                .map_err(|_| Error::Unsupported("invalid locked uv version"))?;
            if !requirement.matches(&version) {
                return Err(Error::Unsupported(
                    "locked uv does not satisfy native declarations",
                ));
            }
            Some(plan)
        }
        None => None,
    };
    let python = match snapshot.python_requirement()? {
        Some(requirement) => {
            let plan = PythonPlan::from_lock(lock, platform)?;
            let version = semver::Version::parse(&plan.inventory_tool().version)
                .map_err(|_| Error::Unsupported("invalid locked Python version"))?;
            if !requirement.matches(&version) {
                return Err(Error::Unsupported(
                    "locked Python does not satisfy native declarations",
                ));
            }
            Some(plan)
        }
        None => None,
    };
    if node.is_none()
        && uv.is_none()
        && python.is_none()
        && rustup.is_none()
        && rust.is_none()
    {
        return Err(Error::Unsupported("turbo.lock selects no managed tools"));
    }
    Ok(Plans {
        node,
        pnpm,
        npm,
        rustup,
        rust,
        uv,
        python,
    })
}

/// Aggregate results for the caller's report; no paths or versions.
#[derive(Debug, Default)]
pub(super) struct Provisioned {
    pub downloaded_bytes: u64,
    pub installed: usize,
    pub reused: usize,
}

pub(super) fn lock_report(
    previous: Option<&turborepo_setup::lock::Lock>,
    current: &turborepo_setup::lock::Lock,
) -> String {
    let mut lines = vec![format!(
        "turbo.lock {}. Commit turbo.lock to share the exact managed tool selections.",
        if previous.is_some() {
            "updated"
        } else {
            "created"
        }
    )];
    for (id, tool) in current.tools() {
        let old = previous.and_then(|lock| lock.tools().get(id));
        if old == Some(tool) {
            continue;
        }
        lines.push(match old {
            Some(old) if old.version != tool.version => {
                format!("{id}: {} -> {}", old.version, tool.version)
            }
            Some(_) => format!("{id}: {} (selection changed)", tool.version),
            None => format!("{id}: locked {}", tool.version),
        });
    }
    for (id, tool) in previous.into_iter().flat_map(|lock| lock.tools()) {
        if !current.tools().contains_key(id) {
            lines.push(format!("{id}: {} -> removed from lock", tool.version));
        }
    }
    lines.join("\n")
}

pub(super) fn run(
    discovery: &super::root::Discovery,
    snapshot: Snapshot,
    request: &super::SetupRequest,
    transports: Option<Transports>,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<Provisioned, Error> {
    let (force, offline) = (request.force, request.offline);
    let resolution_mode = if request.update_lock {
        turborepo_setup::lock::reconcile::Mode::Refresh
    } else {
        turborepo_setup::lock::reconcile::Mode::Local
    };
    // Force and offline never enter resolution or lock publication.
    let write = request.lock == super::LockMode::Write && !force && !offline;
    let root = discovery.snapshot_root()?.as_std_path();
    let host = platform()?;
    if request.lock == super::LockMode::Frozen {
        super::lock_diagnostics::frozen(&snapshot)?;
    }
    if offline {
        let lock = snapshot.previous_lock().ok_or(Error::Unsupported(
            "offline setup requires turbo.lock; run turbo setup --tools-only without --offline",
        ))?;
        if !lock.matches_native(snapshot.declarations())? {
            return Err(Error::Unsupported(
                "offline setup cannot resolve changed tool declarations; run turbo setup \
                 --tools-only without --offline",
            ));
        }
    }
    let declared = snapshot.declarations();
    let js_manager = declared.contains_key("npm") || declared.contains_key("pnpm");
    if write
        && !declared.contains_key("node")
        && (js_manager || !["rustup", "uv"].iter().any(|id| declared.contains_key(*id)))
    {
        return Err(Error::Unsupported(
            if snapshot.ecosystems().python && !js_manager {
                "a native Node or uv declaration is required"
            } else {
                "a native Node declaration is required (or a root rust-toolchain file with \
                 experimentalCargoWorkspaces)"
            },
        ));
    }
    // Ordinary setup validates matching pins before its no-op writer. Explicit
    // refresh may repair invalid native selections within this supported cohort;
    // the candidate callback still gates EVERY new selection before any writer.
    if write && let Some(lock) = snapshot.previous_lock() {
        if resolution_mode == turborepo_setup::lock::reconcile::Mode::Refresh
            && lock
                .tools()
                .keys()
                .any(|id| !SUPPORTED.contains(&id.as_str()))
        {
            return Err(Error::Unsupported(
                "only managed Node, npm, pnpm, Rust, uv, and Python are supported",
            ));
        }
        if resolution_mode == turborepo_setup::lock::reconcile::Mode::Local
            && lock.matches_native(snapshot.declarations())?
        {
            plans(&snapshot, lock)?;
        }
    }
    // User state is checked before the first writer or download, but after the
    // read-only lock/plan diagnostics above and below, which never need it.
    let trust_base = transports
        .as_ref()
        .map(Transports::trust_base)
        .transpose()?;
    let open_trust = || -> Result<super::trust::TrustStore, Error> {
        let base = trust_base
            .clone()
            .or_else(dirs_next::data_local_dir)
            .ok_or_else(|| io::Error::other("cannot locate setup user state directory"))?;
        let trust = super::trust::TrustStore::new(&base, root)?;
        trust.preflight()?;
        Ok(trust)
    };
    let mut trust = None;
    let mut transports = transports;
    let snapshot = if write {
        use turborepo_setup::lock::{WriteOutcome, reconcile};
        let trust = trust.insert(open_trust()?);
        let check = || {
            trust.preflight()?;
            preflight()?;
            revalidate(discovery)?;
            storage(root)?;
            Ok::<_, Error>(())
        };
        check()?;
        snapshot.ensure_current()?;
        let policy = preflight()?;
        let sources = transports
            .take()
            .map(Ok)
            .unwrap_or_else(|| Transports::official(&policy))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let outcome = reconcile::reconcile_checked(
            &snapshot,
            resolution_mode,
            false,
            |request| {
                let failed =
                    |e: &dyn std::fmt::Display| reconcile::Error::Resolution(e.to_string());
                check().map_err(|e| failed(&e))?;
                // Each ecosystem resolves only its own changed declarations;
                // reconcile still checks every unaffected selection's bytes.
                let rust =
                    turborepo_setup::rust_resolution::resolve_request(&request, &sources.rustup);
                let mut tools = runtime.block_on(rust).map_err(|e| failed(&e))?;
                // JavaScript plus, with Python workspaces, uv and CPython.
                let declared = request.snapshot().declarations();
                if ["node", "npm", "pnpm", "uv", "python"]
                    .iter()
                    .any(|id| declared.contains_key(*id))
                {
                    let native = runtime
                        .block_on(turborepo_setup::native_resolution::resolve(
                            request,
                            turborepo_setup::native_resolution::Transports {
                                node: &sources.node,
                                registry: &sources.registry,
                                uv: &sources.uv,
                                python: &sources.python,
                                platform: platform().map_err(|e| failed(&e))?,
                            },
                        ))
                        .map_err(|e| failed(&e))?;
                    tools.extend(native.tools().clone());
                }
                Ok(turborepo_setup::lock::Lock::new(
                    turborepo_setup::lock::Document {
                        schema_version: turborepo_setup::lock::SCHEMA_VERSION,
                        tools,
                    },
                )?)
            },
            |candidate| {
                plans(&snapshot, candidate)
                    .map(drop)
                    .map_err(io::Error::other)
            },
            || check().map_err(io::Error::other),
        )?;
        // Preserve the original root/sources; accepting a freshly captured lock
        // here would hide a competing candidate or concurrent declaration edit.
        check()?;
        let published = snapshot.after_publication(&outcome.lock)?;
        if outcome.publication == Some(WriteOutcome::Written) {
            println!("{}", lock_report(snapshot.previous_lock(), &outcome.lock));
        }
        transports = Some(sources);
        published
    } else {
        // Frozen mode names stale declarations instead of a generic mismatch.
        if lock_mode == super::LockMode::Frozen && snapshot.previous_lock().is_some() {
            use turborepo_setup::lock::reconcile;
            reconcile::reconcile(&snapshot, reconcile::Mode::Frozen, false, |_| {
                Err(reconcile::Error::Resolution(
                    "frozen setup never resolves".into(),
                ))
            })?;
        }
        snapshot
    };
    let lock = snapshot
        .previous_lock()
        .ok_or(Error::Unsupported(if force {
            "locked provisioning requires turbo.lock"
        } else {
            "frozen mode requires turbo.lock"
        }))?;
    let Plans {
        node,
        pnpm,
        npm,
        rustup,
        rust,
        uv,
        python,
    } = plans(&snapshot, lock)?;
    // Offline transports exist for Node and the npm registry only; rustup, uv
    // and CPython downloads have no offline mode yet.
    if offline && (rustup.is_some() || rust.is_some() || uv.is_some() || python.is_some()) {
        return Err(Error::Unsupported(
            "offline setup for managed Rust, uv or Python",
        ));
    }
    let trust = match trust {
        Some(trust) => trust,
        None => open_trust()?,
    };
    let desired: Vec<_> = node
        .iter()
        .map(NodePlan::inventory_tool)
        .chain(pnpm.iter().map(PnpmPlan::inventory_tool))
        .chain(npm.iter().map(NpmPlan::inventory_tool))
        .chain(rustup.iter().map(RustupPlan::inventory_tool))
        .chain(rust.iter().map(RustPlan::inventory_tool))
        .chain(uv.iter().map(UvPlan::inventory_tool))
        .chain(python.iter().map(PythonPlan::inventory_tool))
        .cloned()
        .collect();
    let policy = preflight()?;
    let transports = transports
        .map(Ok)
        .unwrap_or_else(|| Transports::official(&policy))?;
    let transports = if offline {
        transports.offline()
    } else {
        transports
    };
    revalidate(discovery)?;
    storage(root)?;
    snapshot.ensure_current()?;
    // Fixed acquisition order: writer, then Store. Both remain held through the
    // final precondition and manifest selection, including unchanged results.
    let guard = snapshot.guard()?;
    storage(root)?;
    let mut store = Store::open(root)?;
    storage(root)?;
    // Locks may have waited behind another transaction: recheck before traffic.
    preflight()?;
    revalidate(discovery)?;
    snapshot.check_guard(&guard)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // Repo inventory can prove health, never provenance. A missing/stale user
    // record forces the entire cohort through verified artifact preparation.
    let recorded = trust.allows_reuse(&store)?;
    let replace = force || !recorded;
    let preparation = if replace {
        turborepo_setup::Preparation::Fresh
    } else {
        turborepo_setup::Preparation::IfNeeded
    };
    // Offline misses are collected, not short-circuited, so the report names
    // every missing artifact. Nothing is published unless all are present.
    // Offline --force reinstalls from verified cached bytes, or else from the
    // healthy installed tree the adapter would reuse, but only when the
    // external trust record proves that tree's provenance.
    let mut missing = Vec::new();
    let mut reused = Vec::new();
    let (prepared_node, prepared_pnpm, prepared_npm, prepared_rustup, prepared_uv, prepared_python) =
        runtime.block_on(async {
            use turborepo_setup::{Preparation::IfNeeded, node_provision::Error::Offline};
            let transport = &transports.node;
            let node = match &node {
                Some(plan) => match plan.prepare(&store, &desired, transport, preparation).await {
                    Err(Offline) if force && recorded => {
                        match plan.prepare(&store, &desired, transport, IfNeeded).await {
                            Ok(None) => {
                                reused.push(plan.inventory_tool().clone());
                                None
                            }
                            Err(Offline) => {
                                missing.push("node");
                                None
                            }
                            result => result?,
                        }
                    }
                    Err(Offline) => {
                        missing.push("node");
                        None
                    }
                    result => result?,
                },
                None => None,
            };
            preflight()?;
            revalidate(discovery)?;
            snapshot.check_guard(&guard)?;
            use turborepo_setup::pnpm_provision::Error::Offline as PnpmOffline;
            let transport = &transports.pnpm;
            let pnpm = match &pnpm {
                Some(plan) => match plan.prepare(&store, &desired, transport, preparation).await {
                    Err(PnpmOffline) if force && recorded => {
                        match plan.prepare(&store, &desired, transport, IfNeeded).await {
                            Ok(None) => {
                                reused.push(plan.inventory_tool().clone());
                                None
                            }
                            Err(PnpmOffline) => {
                                missing.push("pnpm");
                                None
                            }
                            result => result?,
                        }
                    }
                    Err(PnpmOffline) => {
                        missing.push("pnpm");
                        None
                    }
                    result => result?,
                },
                None => None,
            };
            // npm and pnpm use the same exact, integrity-verified registry
            // transport; the plan determines package identity, never PATH.
            let npm = match &npm {
                Some(plan) => match plan.prepare(&store, &desired, transport, preparation).await {
                    Err(PnpmOffline) if force && recorded => {
                        match plan.prepare(&store, &desired, transport, IfNeeded).await {
                            Ok(None) => {
                                reused.push(plan.inventory_tool().clone());
                                None
                            }
                            Err(PnpmOffline) => {
                                missing.push("npm");
                                None
                            }
                            result => result?,
                        }
                    }
                    Err(PnpmOffline) => {
                        missing.push("npm");
                        None
                    }
                    result => result?,
                },
                None => None,
            };
            preflight()?;
            revalidate(discovery)?;
            snapshot.check_guard(&guard)?;
            let rustup = match &rustup {
                Some(plan) => {
                    plan.prepare(&store, &desired, &transports.rustup, preparation)
                        .await?
                }
                None => None,
            };
            preflight()?;
            revalidate(discovery)?;
            snapshot.check_guard(&guard)?;
            let uv = match &uv {
                Some(plan) => {
                    plan.prepare(&store, &desired, &transports.uv, preparation)
                        .await?
                }
                None => None,
            };
            preflight()?;
            revalidate(discovery)?;
            snapshot.check_guard(&guard)?;
            let python = match &python {
                Some(plan) => {
                    plan.prepare(&store, &desired, &transports.python, preparation)
                        .await?
                }
                None => None,
            };
            Ok::<_, Error>((node, pnpm, npm, rustup, uv, python))
        })?;
    if !missing.is_empty() {
        return Err(Error::OfflineMissing(missing_artifacts(
            lock, host, &missing,
        )));
    }
    preflight()?;
    revalidate(discovery)?;
    snapshot.check_guard(&guard)?;
    let prepared_rust = match (&rust, &rustup) {
        (Some(plan), Some(rustup)) => {
            // The verified installer is the only rustup this toolchain sees.
            let scratch = tempfile::Builder::new()
                .prefix(".rustup-")
                .tempdir_in(root.join(".turbo/tools"))?;
            let rustup =
                rustup.executable(&store, &desired, prepared_rustup.as_ref(), scratch.path())?;
            plan.prepare(&store, &desired, &rustup, preparation)?
        }
        _ => None,
    };
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| {
        let missing = turborepo_tool_install::Error::InvalidInventory;
        match tool.id.as_str() {
            "node" => prepared_node
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "pnpm" => prepared_pnpm
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "npm" => prepared_npm
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "rustup" => prepared_rustup
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "rust" => prepared_rust
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "uv" => prepared_uv
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "python" => prepared_python
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            _ => Err(missing),
        }
    };
    let before_publish = || {
        trust.preflight()?;
        storage(root).map_err(io::Error::other)?;
        preflight().map_err(io::Error::other)?;
        revalidate(discovery).map_err(io::Error::other)?;
        snapshot.check_guard(&guard).map_err(io::Error::other)?;
        Ok(())
    };
    if replace {
        store.force_reconcile_reusing_checked(&desired, &reused, stage, before_publish)?;
    } else {
        store.reconcile_checked(&desired, stage, before_publish)?;
    }
    // A published generation without this second, external publication is not
    // trusted. On failure, rerunning setup freshly verifies it before repair.
    before_publish()?;
    trust.publish(&store)?;
    let mut provisioned = Provisioned {
        downloaded_bytes: prepared_node
            .as_ref()
            .map_or(0, |node| node.downloaded_bytes())
            + prepared_pnpm
                .as_ref()
                .map_or(0, |pnpm| pnpm.downloaded_bytes())
            + prepared_npm
                .as_ref()
                .map_or(0, |npm| npm.downloaded_bytes()),
        ..Provisioned::default()
    };
    for tool in &desired {
        let from_cache = match tool.id.as_str() {
            "node" => prepared_node.as_ref().map(|node| node.from_cache()),
            "pnpm" => prepared_pnpm.as_ref().map(|pnpm| pnpm.from_cache()),
            "npm" => prepared_npm.as_ref().map(|npm| npm.from_cache()),
            "rust" => prepared_rust.as_ref().map(|_| false),
            "uv" => prepared_uv.as_ref().map(|_| false),
            "python" => prepared_python.as_ref().map(|_| false),
            _ => prepared_rustup.as_ref().map(|_| false),
        };
        if from_cache.is_none() {
            provisioned.reused += 1;
        } else {
            provisioned.installed += 1;
        }
        println!(
            "{} {}: {}",
            tool.id,
            tool.version,
            match from_cache {
                None if force => "reinstalled from the verified installation",
                None => "reused",
                Some(false) => "installed",
                Some(true) => "installed from the shared download cache",
            }
        );
    }
    if offline {
        println!("Offline: used only installed tools and verified cached artifacts.");
    }
    if let Some(npm) = lock.tools().get("npm")
        && matches!(
            npm.installation,
            turborepo_setup::lock::Installation::Bundled { .. }
        )
    {
        println!("npm {}: provided by managed Node (bundled)", npm.version);
    }
    for line in
        super::cleanup::after_provisioning(&mut store, transports.cache.as_ref(), lock, host)
    {
        println!("{line}");
    }
    if turborepo_setup::writer_storage::outside_git_repository(root)? {
        println!("No Git repository found; skipped checking that .turbo/ is Git-ignored.");
    }
    Ok(provisioned)
}
