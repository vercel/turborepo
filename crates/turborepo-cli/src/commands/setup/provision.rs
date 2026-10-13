//! Local reconciliation followed by locked tool provisioning.
//! Force reinstalls committed selections without resolution or lock writes.
//! Offline uses only healthy installs and verified cached artifacts.
//! No probes, dependency execution or activation.
//! Declared additional tools install through the generic artifact adapter.
use std::{fs, io, path::Path, process::Command};

use turborepo_setup::{
    artifact_cache::ArtifactCache,
    artifact_provision::{ArtifactPlan, ArtifactTransport},
    bun_provision::{BunPlan, BunTransport},
    go_provision::GoPlan,
    go_resolution::GoTransport,
    lock::{Installation, Platform, Snapshot, additional},
    mirror::Mirrors,
    node_provision::{NodePlan, NodeTransport},
    npm_provision::NpmPlan,
    package_manager::Manager,
    pnpm_provision::{PnpmPlan, PnpmTransport},
    python_provision::PythonPlan,
    python_resolution::PythonTransport,
    rust_provision::RustPlan,
    rustup_provision::{RustupPlan, RustupTransport},
    source_policy::OfficialSourcePolicy,
    uv_provision::UvPlan,
    uv_resolution::UvTransport,
    yarn_provision::YarnPlan,
};
use turborepo_tool_install::Store;
use turborepo_turbo_json::SetupMirrors;

use super::Error;

const SUPPORTED: [&str; 10] = [
    "node", "npm", "pnpm", "yarn", "bun", "rust", "rustup", "uv", "python", "go",
];

/// Built-in adapters by canonical ID, plus declared additional tools.
fn supported(id: &str, tool: &turborepo_setup::lock::Tool) -> bool {
    SUPPORTED.contains(&id) || tool.adapter == additional::ADAPTER
}

pub(super) struct Transports {
    pub node: NodeTransport,
    pub pnpm: PnpmTransport,
    pub registry: turborepo_setup::registry_resolution::RegistryTransport,
    #[cfg(any(test, feature = "test-support"))]
    pub trust_base: Option<std::path::PathBuf>,
    /// The shared download cache the transports use, cleaned up after setup.
    pub cache: Option<ArtifactCache>,
    pub rustup: RustupTransport,
    pub uv: UvTransport,
    pub python: PythonTransport,
    pub go: GoTransport,
    pub bun: BunTransport,
    /// Fixture override; otherwise built from the locked artifact origins.
    pub artifact: Option<ArtifactTransport>,
}
impl Transports {
    /// Official sources, unless `TURBO_SETUP_*` variables or the root
    /// `setup.mirrors` policy select a mirror, in that order.
    pub fn official(policy: &OfficialSourcePolicy, mirrors: &SetupMirrors) -> Result<Self, Error> {
        // Verified archive bytes are shared through the user's cache directory;
        // extraction and activation stay in each repository's .turbo/tools.
        let cache = dirs_next::cache_dir().and_then(|base| ArtifactCache::new(&base));
        Self::configured(policy, &Mirrors::configured(mirrors)?, cache)
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
        let bun = match &mirrors.bun {
            Some(mirror) => BunTransport::mirrored(policy, mirror)?,
            None => BunTransport::official(policy)?,
        };
        let uv = match &mirrors.uv {
            Some(mirror) => UvTransport::mirrored(policy, mirror)?,
            None => UvTransport::official(policy)?,
        };
        let python = match &mirrors.python {
            Some(mirror) => PythonTransport::mirrored(policy, mirror)?,
            None => PythonTransport::official(policy)?,
        };
        let go = match &mirrors.go {
            Some(mirror) => GoTransport::mirrored(policy, mirror)?,
            None => GoTransport::official(policy)?,
        };
        // Also the private rustup's distribution server.
        let rustup = match &mirrors.rust {
            Some(mirror) => RustupTransport::mirrored(policy, mirror)?,
            None => RustupTransport::official(policy)?,
        };
        Ok(Self {
            node: node.with_cache(cache.clone()),
            pnpm: registry()?.with_cache(cache.clone()),
            registry: registry()?,
            #[cfg(any(test, feature = "test-support"))]
            trust_base: None,
            cache,
            rustup,
            uv,
            python,
            go,
            bun,
            artifact: None,
        })
    }

    pub(super) fn trust_base(&self) -> Result<std::path::PathBuf, io::Error> {
        #[cfg(any(test, feature = "test-support"))]
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
            #[cfg(any(test, feature = "test-support"))]
            trust_base: self.trust_base,
            cache: self.cache,
            rustup: self.rustup,
            uv: self.uv,
            python: self.python,
            go: self.go,
            bun: self.bun,
            artifact: self.artifact,
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
        ("windows", "x86_64") => windows_x64_platform(windows_host_machine()),
        ("windows", "aarch64") => Err(Error::Unsupported(WINDOWS_ARM64)),
        _ => Err(Error::Unsupported(
            "managed promotion requires macOS, GNU Linux or Windows x64",
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

const WINDOWS_ARM64: &str = "managed setup is not qualified for Windows arm64, including x64 \
                             turbo under emulation; use WSL or a Windows x64 machine";

/// Releases ship the x64 binary as the Windows arm64 copy, so an x64 build
/// must still check the host. Native arm64 is unqualified and never borrows
/// x64 artifacts; an unidentified host fails closed.
pub(super) fn windows_x64_platform(host_machine: Option<u16>) -> Result<Platform, Error> {
    const AMD64: u16 = 0x8664;
    const ARM64: u16 = 0xAA64;
    match host_machine {
        Some(AMD64) => Ok(Platform::WindowsX64),
        Some(ARM64) => Err(Error::Unsupported(WINDOWS_ARM64)),
        _ => Err(Error::Unsupported(
            "managed setup could not confirm a Windows x64 host",
        )),
    }
}

/// The host's native machine (`IMAGE_FILE_MACHINE_*`), not this process's.
#[cfg(windows)]
pub(super) fn windows_host_machine() -> Option<u16> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    let (mut process, mut native) = (0, 0);
    // SAFETY: the current-process pseudo-handle is always valid, and both
    // out-pointers are live locals.
    let ok = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) } != 0;
    ok.then_some(native)
}

#[cfg(not(windows))]
pub(super) fn windows_host_machine() -> Option<u16> {
    None
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
    pub go: Option<GoPlan>,
    pub yarn: Option<YarnPlan>,
    /// Bun runs the repository's scripts itself; it needs no Node.
    pub bun: Option<BunPlan>,
    pub artifacts: Vec<ArtifactPlan>,
}

pub(super) fn plans(
    snapshot: &Snapshot,
    lock: &turborepo_setup::lock::Lock,
) -> Result<Plans, Error> {
    let platform = platform()?;
    super::lock_diagnostics::matching(snapshot, lock)?;
    let drift = snapshot.additional_drift(lock);
    if !drift.is_empty() {
        return Err(Error::AdditionalToolDrift(drift));
    }
    if lock.tools().is_empty() || lock.tools().iter().any(|(id, tool)| !supported(id, tool)) {
        return Err(Error::Unsupported(
            "only managed Node, npm, pnpm, Yarn, Bun, Rust, uv, Python, Go, and additional tools \
             are supported",
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
    let (mut pnpm, mut npm, mut yarn, mut bun) = (None, None, None, None);
    if let Some(manager) = snapshot.package_manager()? {
        match (manager.manager, &node) {
            (Manager::Pnpm, Some(node)) => {
                pnpm = Some(PnpmPlan::from_declaration(lock, platform, node, &manager)?);
            }
            (Manager::Npm, Some(node)) => {
                npm = NpmPlan::from_declaration(lock, platform, node, &manager)?;
            }
            (Manager::Yarn, Some(node)) => {
                yarn = Some(YarnPlan::from_declaration(lock, platform, node, &manager)?);
            }
            // Bun runs the repository's scripts itself; it needs no Node.
            (Manager::Bun, _) => {
                bun = Some(BunPlan::from_declaration(lock, platform, &manager)?);
            }
            (Manager::Pnpm | Manager::Npm | Manager::Yarn, None) => {
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
    let go = match (snapshot.go_requirements()?, lock.tools().get("go")) {
        (Some(requirements), Some(tool)) => {
            let version = turborepo_setup::go_discovery::GoVersion::parse(&tool.version);
            if !version.is_some_and(|v| requirements.matches_locked(v)) {
                return Err(Error::Unsupported(
                    "locked Go does not satisfy native declarations",
                ));
            }
            Some(GoPlan::from_lock(lock, platform)?)
        }
        (None, None) => None,
        _ => return Err(Error::Unsupported("turbo.lock Go selection is undeclared")),
    };
    let artifacts = ArtifactPlan::from_lock(lock, platform)?;
    if node.is_none()
        && uv.is_none()
        && python.is_none()
        && rustup.is_none()
        && rust.is_none()
        && go.is_none()
        && bun.is_none()
        && artifacts.is_empty()
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
        go,
        yarn,
        bun,
        artifacts,
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

/// Declarations lock resolution can start from. Setup and `--plan` share this
/// gate and [`resolve`], so a plan proposes exactly what setup would publish;
/// ecosystems that join resolution extend these, never one caller.
pub(super) fn resolvable(snapshot: &Snapshot) -> Result<(), Error> {
    let declared = snapshot.declarations();
    // Package managers other than Bun run on the managed Node.
    let js_manager = ["npm", "pnpm", "yarn"]
        .iter()
        .any(|id| declared.contains_key(*id));
    if !declared.contains_key("node")
        && (js_manager
            || !["rustup", "uv", "go", "bun"]
                .iter()
                .any(|id| declared.contains_key(*id)))
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
    Ok(())
}

/// The one candidate resolver behind setup's lock publication and `--plan`'s
/// proposal. Callers run their own checks before calling it. `snapshot` is the
/// caller's captured snapshot, which Go's completion reads after the request
/// moves into native resolution.
pub(super) fn resolve(
    runtime: &tokio::runtime::Runtime,
    request: turborepo_setup::lock::reconcile::Resolution<'_>,
    sources: &Transports,
    snapshot: &Snapshot,
) -> Result<turborepo_setup::lock::Lock, turborepo_setup::lock::reconcile::Error> {
    use turborepo_setup::lock::reconcile;
    let failed = |e: &dyn std::fmt::Display| reconcile::Error::Resolution(e.to_string());
    // Each ecosystem resolves only its own changed declarations;
    // reconcile still checks every unaffected selection's bytes.
    let go = request.version_ids().contains("go");
    let rust = turborepo_setup::rust_resolution::resolve_request(&request, &sources.rustup);
    let mut tools = runtime.block_on(rust).map_err(|e| failed(&e))?;
    // JavaScript plus, with Python workspaces, uv and CPython.
    let declared = request.snapshot().declarations();
    if ["node", "npm", "pnpm", "yarn", "bun", "uv", "python"]
        .iter()
        .any(|id| declared.contains_key(*id))
    {
        let native = runtime
            .block_on(turborepo_setup::native_resolution::resolve(
                request,
                turborepo_setup::native_resolution::Transports {
                    node: &sources.node,
                    registry: &sources.registry,
                    bun: &sources.bun,
                    uv: &sources.uv,
                    python: &sources.python,
                    platform: platform().map_err(|e| failed(&e))?,
                },
            ))
            .map_err(|e| failed(&e))?;
        tools.extend(native.tools().clone());
    }
    let cohort = turborepo_setup::lock::Lock::new(turborepo_setup::lock::Document {
        schema_version: turborepo_setup::lock::SCHEMA_VERSION,
        tools,
    })?;
    // Go composes onto the other ecosystems' cohort.
    runtime
        .block_on(turborepo_setup::go_resolution::complete(
            snapshot,
            go,
            cohort,
            &sources.go,
        ))
        .map_err(|e| failed(&e))
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
    if write {
        resolvable(&snapshot)?;
    }
    // Ordinary setup validates matching pins before its no-op writer. Explicit
    // refresh may repair invalid native selections within this supported cohort;
    // the candidate callback still gates EVERY new selection before any writer.
    if write && let Some(lock) = snapshot.previous_lock() {
        if resolution_mode == turborepo_setup::lock::reconcile::Mode::Refresh
            && lock.tools().iter().any(|(id, tool)| !supported(id, tool))
        {
            return Err(Error::Unsupported(
                "only managed Node, npm, pnpm, Yarn, Bun, Rust, uv, Python, Go, and additional \
                 tools are supported",
            ));
        }
        if resolution_mode == turborepo_setup::lock::reconcile::Mode::Local
            && lock.matches_native(snapshot.declarations())?
            && snapshot.matches_additional(lock)
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
            .unwrap_or_else(|| Transports::official(&policy, discovery.policy().mirrors()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let outcome = reconcile::reconcile_checked(
            &snapshot,
            resolution_mode,
            false,
            |request| {
                check().map_err(|e| reconcile::Error::Resolution(e.to_string()))?;
                resolve(&runtime, request, &sources, &snapshot)
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
        if request.lock == super::LockMode::Frozen && snapshot.previous_lock().is_some() {
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
        go,
        yarn,
        bun,
        artifacts,
    } = plans(&snapshot, lock)?;
    // Offline transports exist for Node and the npm registry only; rustup, uv,
    // CPython, Go, Bun and additional tool downloads have no offline mode yet.
    if offline
        && (!artifacts.is_empty()
            || rustup.is_some()
            || rust.is_some()
            || uv.is_some()
            || python.is_some()
            || go.is_some()
            || bun.is_some())
    {
        return Err(Error::Unsupported(
            "offline setup for managed Bun, Rust, uv, Python, Go or additional tools",
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
        .chain(go.iter().map(GoPlan::inventory_tool))
        .chain(yarn.iter().map(YarnPlan::inventory_tool))
        .chain(bun.iter().map(BunPlan::inventory_tool))
        .chain(artifacts.iter().map(ArtifactPlan::inventory_tool))
        .cloned()
        .collect();
    let policy = preflight()?;
    let mut transports = transports
        .map(Ok)
        .unwrap_or_else(|| Transports::official(&policy, discovery.policy().mirrors()))?;
    let artifact_transport = match transports.artifact.take() {
        Some(transport) => Some(transport),
        None if artifacts.is_empty() => None,
        None => Some(ArtifactTransport::new(&policy, &artifacts)?),
    };
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
    let (
        prepared_node,
        prepared_pnpm,
        prepared_npm,
        prepared_rustup,
        prepared_uv,
        prepared_python,
        prepared_go,
        prepared_yarn,
        prepared_bun,
        prepared_artifacts,
    ) = runtime.block_on(async {
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
        // Yarn shares the registry transport and preparation type with pnpm.
        let yarn = match &yarn {
            Some(plan) => match plan.prepare(&store, &desired, transport, preparation).await {
                Err(PnpmOffline) if force && recorded => {
                    match plan.prepare(&store, &desired, transport, IfNeeded).await {
                        Ok(None) => {
                            reused.push(plan.inventory_tool().clone());
                            None
                        }
                        Err(PnpmOffline) => {
                            missing.push("yarn");
                            None
                        }
                        result => result?,
                    }
                }
                Err(PnpmOffline) => {
                    missing.push("yarn");
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
        preflight()?;
        revalidate(discovery)?;
        snapshot.check_guard(&guard)?;
        let go = match &go {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.go, preparation)
                    .await?
            }
            None => None,
        };
        preflight()?;
        revalidate(discovery)?;
        snapshot.check_guard(&guard)?;
        let bun = match &bun {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.bun, preparation)
                    .await?
            }
            None => None,
        };
        let mut prepared = Vec::new();
        if let Some(transport) = &artifact_transport {
            for plan in &artifacts {
                preflight()?;
                revalidate(discovery)?;
                snapshot.check_guard(&guard)?;
                prepared.push(
                    plan.prepare(&store, &desired, transport, preparation)
                        .await?,
                );
            }
        }
        Ok::<_, Error>((node, pnpm, npm, rustup, uv, python, go, yarn, bun, prepared))
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
            plan.prepare(&store, &desired, &rustup, &transports.rustup, preparation)?
        }
        _ => None,
    };
    let additional = |tool: &turborepo_tool_install::Tool| {
        let index = artifacts.iter().position(|p| p.inventory_tool() == tool)?;
        Some((&artifacts[index], prepared_artifacts[index].as_ref()))
    };
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| {
        let missing = turborepo_tool_install::Error::InvalidInventory;
        if let Some((_, prepared)) = additional(tool) {
            return prepared.ok_or(missing)?.stage(tool, destination);
        }
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
            "go" => prepared_go
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "yarn" => prepared_yarn
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "bun" => prepared_bun
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
                .map_or(0, |npm| npm.downloaded_bytes())
            + prepared_yarn
                .as_ref()
                .map_or(0, |yarn| yarn.downloaded_bytes()),
        ..Provisioned::default()
    };
    for tool in &desired {
        let source = additional(tool)
            .map(|(plan, _)| format!(" ({})", plan.source()))
            .unwrap_or_default();
        let from_cache = match tool.id.as_str() {
            _ if additional(tool).is_some() => {
                additional(tool).and_then(|(_, prepared)| prepared.map(|_| false))
            }
            "node" => prepared_node.as_ref().map(|node| node.from_cache()),
            "pnpm" => prepared_pnpm.as_ref().map(|pnpm| pnpm.from_cache()),
            "npm" => prepared_npm.as_ref().map(|npm| npm.from_cache()),
            "rust" => prepared_rust.as_ref().map(|_| false),
            "uv" => prepared_uv.as_ref().map(|_| false),
            "python" => prepared_python.as_ref().map(|_| false),
            "go" => prepared_go.as_ref().map(|_| false),
            "yarn" => prepared_yarn.as_ref().map(|yarn| yarn.from_cache()),
            "bun" => prepared_bun.as_ref().map(|_| false),
            _ => prepared_rustup.as_ref().map(|_| false),
        };
        if from_cache.is_none() {
            provisioned.reused += 1;
        } else {
            provisioned.installed += 1;
        }
        println!(
            "{} {}: {}{source}",
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
