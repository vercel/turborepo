//! Local reconciliation followed by locked tools-only provisioning.
//! Force reinstalls committed selections without resolution or lock writes.
//! No probes, dependency execution or activation.
use std::{fs, io, path::Path, process::Command};

use turborepo_setup::{
    go_provision::GoPlan,
    go_resolution::GoTransport,
    lock::{Platform, Snapshot},
    node_provision::{NodePlan, NodeTransport},
    package_manager::Manager,
    pnpm_provision::{PnpmPlan, PnpmTransport},
    source_policy::OfficialSourcePolicy,
};
use turborepo_tool_install::Store;

use super::Error;

pub(super) struct Transports {
    pub node: NodeTransport,
    pub pnpm: PnpmTransport,
    pub registry: turborepo_setup::registry_resolution::RegistryTransport,
    pub go: GoTransport,
}
impl Transports {
    pub fn official(policy: &OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            node: NodeTransport::official(policy)?,
            pnpm: PnpmTransport::official(policy)?,
            registry: turborepo_setup::registry_resolution::RegistryTransport::official(policy)?,
            go: GoTransport::official(policy)?,
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

fn revalidate(discovery: &super::root::Discovery) -> Result<(), Error> {
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
    };
    let ignored = git(&["check-ignore", "--", ".turbo/"])?;
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
    pub go: Option<GoPlan>,
}
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
            "only managed Node, pnpm, and Go are supported",
        ));
    }
    // Matching declarations mean each locked tool is declared, and vice versa.
    let node = (lock.tools().contains_key("node"))
        .then(|| NodePlan::from_lock(lock, platform))
        .transpose()?;
    if let Some(node) = &node {
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
    }
    let pnpm = match (snapshot.package_manager()?, &node) {
        (Some(manager), _) if manager.manager != Manager::Pnpm => {
            return Err(Error::Unsupported("npm override provisioning"));
        }
        (Some(manager), Some(node)) => {
            Some(PnpmPlan::from_declaration(lock, platform, node, &manager)?)
        }
        (Some(_), None) => return Err(Error::Unsupported(NODE_REQUIRED)),
        (None, _) => None,
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
    Ok(Plans { node, pnpm, go })
}

const SUPPORTED: [&str; 3] = ["node", "pnpm", "go"];
const NODE_REQUIRED: &str = "a native Node declaration is required";

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
    // Unsupported native requests must fail before ANY writer, even no-op locks.
    let manager = snapshot.package_manager()?;
    if manager.as_ref().is_some_and(|m| m.manager != Manager::Pnpm) {
        return Err(Error::Unsupported("npm override provisioning"));
    }
    let declared = |id: &str| snapshot.declarations().contains_key(id);
    if write
        && (["npm", "pnpm"].iter().any(|id| declared(id)) || !declared("go"))
        && !declared("node")
    {
        return Err(Error::Unsupported(NODE_REQUIRED));
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
                "only managed Node, pnpm, and Go are supported",
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
                let failed =
                    |e: &dyn std::fmt::Display| reconcile::Error::Resolution(e.to_string());
                check().map_err(|e| failed(&e))?;
                // JS resolves its cohort first; Go composes onto it.
                let go = request.version_ids().contains("go");
                let cohort = if declared("node") {
                    runtime
                        .block_on(turborepo_setup::js_resolution::resolve(
                            request,
                            &sources.node,
                            &sources.registry,
                        ))
                        .map_err(|e| failed(&e))?
                } else {
                    turborepo_setup::lock::Lock::new(turborepo_setup::lock::Document {
                        schema_version: turborepo_setup::lock::SCHEMA_VERSION,
                        tools: Default::default(),
                    })?
                };
                runtime
                    .block_on(turborepo_setup::go_resolution::complete(
                        &snapshot,
                        go,
                        cohort,
                        &sources.go,
                    ))
                    .map_err(|e| failed(&e))
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
        snapshot
    };
    let lock = snapshot
        .previous_lock()
        .ok_or(Error::Unsupported(if force {
            "locked provisioning requires turbo.lock"
        } else {
            "frozen mode requires turbo.lock"
        }))?;
    let Plans { node, pnpm, go } = plans(&snapshot, lock)?;
    let desired: Vec<_> = (node.iter().map(NodePlan::inventory_tool))
        .chain(pnpm.iter().map(PnpmPlan::inventory_tool))
        .chain(go.iter().map(GoPlan::inventory_tool))
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
    let recheck = || {
        preflight()?;
        revalidate(discovery)?;
        snapshot.check_guard(&guard)?;
        Ok::<_, Error>(())
    };
    let (prepared_node, prepared_pnpm, prepared_go) = runtime.block_on(async {
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
        let go = match &go {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.go, preparation)
                    .await?
            }
            None => None,
        };
        Ok::<_, Error>((node, pnpm, go))
    })?;
    let stage = |tool: &turborepo_tool_install::Tool, destination: &Path| {
        let invalid = turborepo_tool_install::Error::InvalidInventory;
        match tool.id.as_str() {
            "node" => prepared_node
                .as_ref()
                .ok_or(invalid)?
                .stage(tool, destination),
            "pnpm" => prepared_pnpm
                .as_ref()
                .ok_or(invalid)?
                .stage(tool, destination),
            "go" => prepared_go
                .as_ref()
                .ok_or(invalid)?
                .stage(tool, destination),
            _ => Err(invalid),
        }
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
                || (tool.id == "go" && prepared_go.is_none())
            {
                "reused"
            } else {
                "installed"
            }
        );
    }
    println!(
        "Dependencies skipped (--tools-only). No tasks run; managed activation is not enabled."
    );
    Ok(0)
}
