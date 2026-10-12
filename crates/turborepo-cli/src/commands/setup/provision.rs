//! Local reconciliation followed by locked tools-only provisioning.
//! Force reinstalls committed selections without resolution or lock writes.
//! No probes, dependency execution or activation.
use std::{fs, io, path::Path, process::Command};

use turborepo_setup::{
    lock::{Platform, Snapshot},
    node_provision::{NodePlan, NodeTransport},
    package_manager::Manager,
    pnpm_provision::{PnpmPlan, PnpmTransport},
    rust_provision::RustPlan,
    rustup_provision::{RustupPlan, RustupTransport},
    source_policy::OfficialSourcePolicy,
};
use turborepo_tool_install::Store;

use super::Error;

const SUPPORTED: [&str; 4] = ["node", "pnpm", "rust", "rustup"];

pub(super) struct Transports {
    pub node: NodeTransport,
    pub pnpm: PnpmTransport,
    pub registry: turborepo_setup::registry_resolution::RegistryTransport,
    pub rustup: RustupTransport,
}
impl Transports {
    pub fn official(policy: &OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            node: NodeTransport::official(policy)?,
            pnpm: PnpmTransport::official(policy)?,
            registry: turborepo_setup::registry_resolution::RegistryTransport::official(policy)?,
            rustup: RustupTransport::official(policy)?,
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
    pub rustup: Option<RustupPlan>,
    pub rust: Option<RustPlan>,
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
    if lock.tools().is_empty()
        || lock
            .tools()
            .keys()
            .any(|id| !SUPPORTED.contains(&id.as_str()))
    {
        return Err(Error::Unsupported(
            "only managed Node, pnpm and Rust are supported",
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
    let pnpm = match (snapshot.package_manager()?, &node) {
        (None, _) => None,
        (Some(manager), _) if manager.manager != Manager::Pnpm => {
            return Err(Error::Unsupported("npm override provisioning"));
        }
        (Some(manager), Some(node)) => {
            Some(PnpmPlan::from_declaration(lock, platform, node, &manager)?)
        }
        (Some(_), None) => {
            return Err(Error::Unsupported(
                "managed pnpm requires a native Node declaration",
            ));
        }
    };
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
    Ok(Plans {
        node,
        pnpm,
        rustup,
        rust,
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
    if write
        && !["node", "rustup"]
            .iter()
            .any(|id| snapshot.declarations().contains_key(*id))
    {
        return Err(Error::Unsupported(
            "a native Node declaration is required (or a root rust-toolchain file with \
             experimentalCargoWorkspaces)",
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
                "only managed Node, pnpm and Rust are supported",
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
                if ["rust", "rustup"]
                    .iter()
                    .any(|id| request.version_ids().contains(*id))
                {
                    return Err(reconcile::Error::Resolution(
                        "resolving Rust toolchain declarations into turbo.lock is not implemented \
                         yet"
                        .into(),
                    ));
                }
                runtime
                    .block_on(turborepo_setup::js_resolution::resolve(
                        request,
                        &sources.node,
                        &sources.registry,
                    ))
                    .map_err(|e| reconcile::Error::Resolution(e.to_string()))
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
    let Plans {
        node,
        pnpm,
        rustup,
        rust,
    } = plans(&snapshot, lock)?;
    let desired: Vec<_> = node
        .iter()
        .map(NodePlan::inventory_tool)
        .chain(pnpm.iter().map(PnpmPlan::inventory_tool))
        .chain(rustup.iter().map(RustupPlan::inventory_tool))
        .chain(rust.iter().map(RustPlan::inventory_tool))
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
    let (prepared_node, prepared_pnpm, prepared_rustup) = runtime.block_on(async {
        let node = match &node {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.node, preparation)
                    .await?
            }
            None => None,
        };
        preflight()?;
        revalidate(discovery)?;
        snapshot.check_guard(&guard)?;
        let pnpm = match &pnpm {
            Some(plan) => {
                plan.prepare(&store, &desired, &transports.pnpm, preparation)
                    .await?
            }
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
        Ok::<_, Error>((node, pnpm, rustup))
    })?;
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
            "rustup" => prepared_rustup
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            "rust" => prepared_rust
                .as_ref()
                .ok_or(missing)?
                .stage(tool, destination),
            _ => Err(missing),
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
        let reused = match tool.id.as_str() {
            "node" => prepared_node.is_none(),
            "pnpm" => prepared_pnpm.is_none(),
            "rust" => prepared_rust.is_none(),
            _ => prepared_rustup.is_none(),
        };
        println!(
            "{} {}: {}",
            tool.id,
            tool.version,
            if reused { "reused" } else { "installed" }
        );
    }
    println!(
        "Dependencies skipped (--tools-only). No tasks run; managed activation is not enabled."
    );
    Ok(0)
}
