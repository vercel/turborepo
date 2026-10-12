//! Local reconciliation followed by locked tool provisioning.
//! Force reinstalls committed selections without resolution or lock writes.
//! No probes, dependency execution or activation. Declared additional tools
//! install through the generic artifact adapter.
use std::{fs, io, path::Path, process::Command};

use turborepo_setup::{
    artifact_provision::{ArtifactPlan, ArtifactTransport},
    go_provision::GoPlan,
    go_resolution::GoTransport,
    lock::{Platform, Snapshot, additional},
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

use super::Error;

pub(super) struct Transports {
    pub node: NodeTransport,
    pub pnpm: PnpmTransport,
    pub registry: turborepo_setup::registry_resolution::RegistryTransport,
    pub uv: UvTransport,
    pub rustup: RustupTransport,
    pub python: PythonTransport,
    #[cfg(test)]
    pub trust_base: Option<std::path::PathBuf>,
    /// Fixture override; otherwise built from the locked artifact origins.
    pub artifact: Option<ArtifactTransport>,
    pub go: GoTransport,
}
impl Transports {
    pub fn official(policy: &OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            node: NodeTransport::official(policy)?,
            pnpm: PnpmTransport::official(policy)?,
            registry: turborepo_setup::registry_resolution::RegistryTransport::official(policy)?,
            uv: UvTransport::official(policy)?,
            rustup: RustupTransport::official(policy)?,
            python: PythonTransport::official(policy)?,
            #[cfg(test)]
            trust_base: None,
            artifact: None,
            go: GoTransport::official(policy)?,
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
}

fn supported(id: &str, tool: &turborepo_setup::lock::Tool) -> bool {
    SUPPORTED.contains(&id) || tool.adapter == additional::ADAPTER
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
    // Exit 128 is a Git failure (for example, not a repository), not a missing
    // ignore rule; editing .gitignore would not help.
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
    pub uv: Option<UvPlan>,
    pub python: Option<PythonPlan>,
    pub rustup: Option<RustupPlan>,
    pub yarn: Option<YarnPlan>,
    pub rust: Option<RustPlan>,
    pub artifacts: Vec<ArtifactPlan>,
    pub go: Option<GoPlan>,
}

const SUPPORTED: [&str; 9] = [
    "node", "npm", "pnpm", "uv", "python", "rust", "rustup", "go", "yarn",
];
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
    if lock.tools().iter().any(|(id, tool)| !supported(id, tool)) {
        return Err(Error::Unsupported(
            "only managed Node, npm, pnpm, uv, Python, Rust, Go, Yarn, and additional tools are \
             supported",
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
    let (mut pnpm, mut npm, mut yarn) = (None, None, None);
    if let Some(manager) = snapshot.package_manager()? {
        let node = node
            .as_ref()
            .ok_or(Error::Unsupported("a native Node declaration is required"))?;
        match manager.manager {
            Manager::Pnpm => {
                pnpm = Some(PnpmPlan::from_declaration(lock, platform, node, &manager)?);
            }
            Manager::Npm => {
                npm = NpmPlan::from_declaration(lock, platform, node, &manager)?;
            }
            Manager::Yarn => {
                yarn = Some(YarnPlan::from_declaration(lock, platform, node, &manager)?);
            }
            _ => return Err(Error::Unsupported("package manager provisioning")),
        }
    }
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
    let rustup = lock
        .tools()
        .contains_key("rustup")
        .then(|| RustupPlan::from_lock(lock, platform))
        .transpose()?;
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
    let artifacts = ArtifactPlan::from_lock(lock, platform)?;
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
    if node.is_none()
        && uv.is_none()
        && python.is_none()
        && rustup.is_none()
        && rust.is_none()
        && artifacts.is_empty()
        && go.is_none()
    {
        return Err(Error::Unsupported("turbo.lock selects no managed tools"));
    }
    Ok(Plans {
        node,
        pnpm,
        npm,
        uv,
        rustup,
        python,
        rust,
        artifacts,
        go,
        yarn,
    })
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
    let js_manager = declared.contains_key("npm") || declared.contains_key("pnpm");
    if !declared.contains_key("node")
        && (js_manager
            || (!declared.contains_key("uv")
                && !declared.contains_key("rustup")
                && !declared.contains_key("go")))
    {
        return Err(Error::Unsupported(
            if snapshot.ecosystems().python && !js_manager {
                "a native Node or uv declaration is required"
            } else {
                "a native Node declaration is required"
            },
        ));
    }
    Ok(())
}

/// The one candidate resolver behind setup's lock publication and `--plan`'s
/// proposal. Callers run their own checks before calling it.
pub(super) fn resolve(
    runtime: &tokio::runtime::Runtime,
    request: turborepo_setup::lock::reconcile::Resolution<'_>,
    sources: &Transports,
) -> Result<turborepo_setup::lock::Lock, turborepo_setup::lock::reconcile::Error> {
    runtime
        .block_on(turborepo_setup::native_resolution::resolve_with_rust(
            request,
            turborepo_setup::native_resolution::Transports {
                node: &sources.node,
                registry: &sources.registry,
                uv: &sources.uv,
                python: &sources.python,
                go: &sources.go,
                platform: platform().map_err(|e| {
                    turborepo_setup::lock::reconcile::Error::Resolution(e.to_string())
                })?,
            },
            &sources.rustup,
        ))
        .map_err(|e| turborepo_setup::lock::reconcile::Error::Resolution(e.to_string()))
}

pub(super) fn run(
    discovery: &super::root::Discovery,
    snapshot: Snapshot,
    lock_mode: super::LockMode,
    resolution_mode: turborepo_setup::lock::reconcile::Mode,
    force: bool,
    transports: Option<Transports>,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<i32, Error> {
    // Force never enters resolution or lock publication, even in local Write.
    let write = lock_mode == super::LockMode::Write && !force;
    let root = discovery.snapshot_root()?.as_std_path();
    platform()?;
    if lock_mode == super::LockMode::Frozen {
        super::lock_diagnostics::frozen(&snapshot)?;
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
                "only managed Node, npm, pnpm, uv, Python, Rust, Go, Yarn, and additional tools \
                 are supported",
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
        // Preserve actionable read-only source/Git diagnostics before checking
        // user state; none of these checks creates a writer or downloads tools.
        preflight()?;
        revalidate(discovery)?;
        storage(root)?;
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
                check().map_err(|e| reconcile::Error::Resolution(e.to_string()))?;
                resolve(&runtime, request, &sources)
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
        if lock_mode == super::LockMode::Frozen
            && let Some(lock) = snapshot.previous_lock()
        {
            let drift = snapshot.additional_drift(lock);
            if !drift.is_empty() {
                return Err(Error::AdditionalToolDrift(drift));
            }
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
        uv,
        rustup,
        python,
        rust,
        artifacts,
        go,
        yarn,
    } = plans(&snapshot, lock)?;
    let trust = match trust {
        Some(trust) => trust,
        None => open_trust()?,
    };
    let mut desired: Vec<_> = [
        node.as_ref().map(NodePlan::inventory_tool),
        pnpm.as_ref().map(PnpmPlan::inventory_tool),
        npm.as_ref().map(NpmPlan::inventory_tool),
        uv.as_ref().map(UvPlan::inventory_tool),
        rustup.as_ref().map(RustupPlan::inventory_tool),
        python.as_ref().map(PythonPlan::inventory_tool),
        rust.as_ref().map(RustPlan::inventory_tool),
        go.as_ref().map(GoPlan::inventory_tool),
        yarn.as_ref().map(YarnPlan::inventory_tool),
    ]
    .into_iter()
    .flatten()
    .cloned()
    .collect();
    desired.extend(artifacts.iter().map(|plan| plan.inventory_tool().clone()));
    let policy = preflight()?;
    let mut transports = transports
        .map(Ok)
        .unwrap_or_else(|| Transports::official(&policy))?;
    let artifact_transport = match transports.artifact.take() {
        Some(transport) => Some(transport),
        None if artifacts.is_empty() => None,
        None => Some(ArtifactTransport::new(&policy, &artifacts)?),
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
    let (
        prepared_node,
        prepared_pnpm,
        prepared_npm,
        prepared_uv,
        prepared_python,
        prepared_rustup,
        prepared_artifacts,
        prepared_go,
        prepared_yarn,
    ) = runtime.block_on(async {
        let recheck = || {
            preflight()?;
            revalidate(discovery)?;
            snapshot.check_guard(&guard)?;
            Ok::<_, Error>(())
        };
        let node = match &node {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.node, preparation)
                    .await?
            }
            None => None,
        };
        recheck()?;
        let pnpm = match &pnpm {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.pnpm, preparation)
                    .await?
            }
            None => None,
        };
        recheck()?;
        let npm = match &npm {
            Some(plan) => {
                // npm and pnpm share the exact integrity-verified registry transport.
                plan.prepare(&store, &desired, &transports.pnpm, preparation)
                    .await?
            }
            None => None,
        };
        recheck()?;
        let uv = match &uv {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.uv, preparation)
                    .await?
            }
            None => None,
        };
        recheck()?;
        let python = match &python {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.python, preparation)
                    .await?
            }
            None => None,
        };
        recheck()?;
        let rustup = match &rustup {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.rustup, preparation)
                    .await?
            }
            None => None,
        };
        let mut prepared = Vec::new();
        if let Some(transport) = &artifact_transport {
            for plan in &artifacts {
                recheck()?;
                prepared.push(
                    plan.prepare(&store, &desired, transport, preparation)
                        .await?,
                );
            }
        }
        recheck()?;
        let go = match &go {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.go, preparation)
                    .await?
            }
            None => None,
        };
        recheck()?;
        let yarn = match &yarn {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.pnpm, preparation)
                    .await?
            }
            None => None,
        };
        Ok::<_, Error>((node, pnpm, npm, uv, python, rustup, prepared, go, yarn))
    })?;
    preflight()?;
    revalidate(discovery)?;
    snapshot.check_guard(&guard)?;
    trust.preflight()?;
    let prepared_rust = match (&rust, &rustup) {
        (Some(plan), Some(rustup)) => {
            let scratch = tempfile::Builder::new()
                .prefix(".rustup-")
                .tempdir_in(root.join(".turbo/tools"))?;
            let rustup =
                rustup.executable(&store, &desired, prepared_rustup.as_ref(), scratch.path())?;
            plan.prepare(&store, &desired, &rustup, preparation)?
        }
        _ => None,
    };
    let additional = |tool: &turborepo_tool_install::Tool| {
        let index = artifacts.iter().position(|p| p.inventory_tool() == tool)?;
        Some((&artifacts[index], prepared_artifacts[index].as_ref()))
    };
    let missing = || turborepo_tool_install::Error::InvalidInventory;
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| {
        if let Some((_, prepared)) = additional(tool) {
            return prepared.ok_or_else(missing)?.stage(tool, destination);
        }
        match tool.id.as_str() {
            "node" => prepared_node
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "pnpm" => prepared_pnpm
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "npm" => prepared_npm
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "uv" => prepared_uv
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "python" => prepared_python
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "rustup" => prepared_rustup
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "rust" => prepared_rust
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "yarn" => prepared_yarn
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            "go" => prepared_go
                .as_ref()
                .ok_or_else(missing)?
                .stage(tool, destination),
            _ => Err(missing()),
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
        store.force_reconcile_checked(&desired, stage, before_publish)?;
    } else {
        store.reconcile_checked(&desired, stage, before_publish)?;
    }
    // A published generation without this second, external publication is not
    // trusted. On failure, rerunning setup freshly verifies it before repair.
    before_publish()?;
    trust.publish(&store)?;
    for tool in &desired {
        let (reused, source) = match additional(tool) {
            Some((plan, prepared)) => (prepared.is_none(), format!(" ({})", plan.source())),
            None => (
                (tool.id == "node" && prepared_node.is_none())
                    || (tool.id == "pnpm" && prepared_pnpm.is_none())
                    || (tool.id == "npm" && prepared_npm.is_none())
                    || (tool.id == "uv" && prepared_uv.is_none())
                    || (tool.id == "rustup" && prepared_rustup.is_none())
                    || (tool.id == "python" && prepared_python.is_none())
                    || (tool.id == "rust" && prepared_rust.is_none())
                    || (tool.id == "yarn" && prepared_yarn.is_none())
                    || (tool.id == "go" && prepared_go.is_none()),
                String::new(),
            ),
        };
        let status = if reused { "reused" } else { "installed" };
        println!("{} {}: {status}{source}", tool.id, tool.version);
    }
    if let Some(npm) = lock.tools().get("npm")
        && matches!(
            npm.installation,
            turborepo_setup::lock::Installation::Bundled { .. }
        )
    {
        println!("npm {}: provided by managed Node (bundled)", npm.version);
    }
    Ok(0)
}
