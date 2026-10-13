//! Local reconciliation followed by locked tools-only provisioning.
//! Force reinstalls committed selections without resolution or lock writes.
//! No probes, dependency execution or activation.
use std::{fs, io, path::Path, process::Command};

use turborepo_setup::{
    lock::{Platform, Snapshot},
    node_provision::{NodePlan, NodeTransport},
    npm_provision::NpmPlan,
    package_manager::Manager,
    pnpm_provision::{PnpmPlan, PnpmTransport},
    source_policy::OfficialSourcePolicy,
    uv_provision::UvPlan,
    uv_resolution::UvTransport,
};
use turborepo_tool_install::Store;

use super::Error;

pub(super) struct Transports {
    pub node: NodeTransport,
    pub pnpm: PnpmTransport,
    pub registry: turborepo_setup::registry_resolution::RegistryTransport,
    pub uv: UvTransport,
}
impl Transports {
    pub fn official(policy: &OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            node: NodeTransport::official(policy)?,
            pnpm: PnpmTransport::official(policy)?,
            registry: turborepo_setup::registry_resolution::RegistryTransport::official(policy)?,
            uv: UvTransport::official(policy)?,
        })
    }
}

pub(super) fn platform() -> Result<Platform, Error> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok(Platform::MacosX64),
        ("macos", "aarch64") => Ok(Platform::MacosArm64),
        ("linux", "x86_64") if cfg!(target_env = "gnu") => Ok(Platform::LinuxX64Gnu),
        ("linux", "aarch64") if cfg!(target_env = "gnu") => Ok(Platform::LinuxArm64Gnu),
        _ => Err(Error::Unsupported(
            "managed promotion requires macOS or GNU Linux",
        )),
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
}

const SUPPORTED: [&str; 4] = ["node", "npm", "pnpm", "uv"];
pub(super) fn plans(
    snapshot: &Snapshot,
    lock: &turborepo_setup::lock::Lock,
) -> Result<Plans, Error> {
    let platform = platform()?;
    if !lock.matches_native(snapshot.declarations())? {
        return Err(Error::Unsupported(
            "turbo.lock declarations changed; run setup locally and commit the updated lock",
        ));
    }
    if lock
        .tools()
        .keys()
        .any(|id| !SUPPORTED.contains(&id.as_str()))
    {
        return Err(Error::Unsupported(
            "only managed Node, npm, pnpm, and uv are supported",
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
    if node.is_none() && uv.is_none() {
        return Err(Error::Unsupported("turbo.lock selects no managed tools"));
    }
    Ok(Plans {
        node,
        pnpm,
        npm,
        uv,
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
    if !declared.contains_key("node") && (js_manager || !declared.contains_key("uv")) {
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
        .block_on(turborepo_setup::native_resolution::resolve(
            request,
            &sources.node,
            &sources.registry,
            &sources.uv,
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
    if write {
        resolvable(&snapshot)?;
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
                "only managed Node, npm, pnpm, and uv are supported",
            ));
        }
        if resolution_mode == turborepo_setup::lock::reconcile::Mode::Local
            && lock.matches_native(snapshot.declarations())?
        {
            plans(&snapshot, lock)?;
        }
    }
    let mut transports = transports;
    let snapshot = if write {
        use turborepo_setup::lock::{WriteOutcome, reconcile};
        let check = || {
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
        uv,
    } = plans(&snapshot, lock)?;
    let desired: Vec<_> = [
        node.as_ref().map(NodePlan::inventory_tool),
        pnpm.as_ref().map(PnpmPlan::inventory_tool),
        npm.as_ref().map(NpmPlan::inventory_tool),
        uv.as_ref().map(UvPlan::inventory_tool),
    ]
    .into_iter()
    .flatten()
    .cloned()
    .collect();
    let policy = preflight()?;
    let transports = transports
        .map(Ok)
        .unwrap_or_else(|| Transports::official(&policy))?;
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
    let preparation = if force {
        turborepo_setup::Preparation::Fresh
    } else {
        turborepo_setup::Preparation::IfNeeded
    };
    let (prepared_node, prepared_pnpm, prepared_npm, prepared_uv) = runtime.block_on(async {
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
        Ok::<_, Error>((node, pnpm, npm, uv))
    })?;
    let missing = || turborepo_tool_install::Error::InvalidInventory;
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| match tool.id.as_str() {
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
        _ => Err(missing()),
    };
    let before_publish = || {
        storage(root).map_err(io::Error::other)?;
        preflight().map_err(io::Error::other)?;
        revalidate(discovery).map_err(io::Error::other)?;
        snapshot.check_guard(&guard).map_err(io::Error::other)?;
        Ok(())
    };
    if force {
        store.force_reconcile_checked(&desired, stage, before_publish)?;
    } else {
        store.reconcile_checked(&desired, stage, before_publish)?;
    }
    for tool in &desired {
        println!(
            "{} {}: {}",
            tool.id,
            tool.version,
            if (tool.id == "node" && prepared_node.is_none())
                || (tool.id == "pnpm" && prepared_pnpm.is_none())
                || (tool.id == "npm" && prepared_npm.is_none())
                || (tool.id == "uv" && prepared_uv.is_none())
            {
                "reused"
            } else {
                "installed"
            }
        );
    }
    if let Some(npm) = lock.tools().get("npm")
        && matches!(
            npm.installation,
            turborepo_setup::lock::Installation::Bundled { .. }
        )
    {
        println!("npm {}: provided by managed Node (bundled)", npm.version);
    }
    println!(
        "Dependencies skipped (--tools-only). No tasks run; managed activation is not enabled."
    );
    Ok(0)
}
