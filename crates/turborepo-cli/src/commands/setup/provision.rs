//! Frozen tools-only executor. No resolution, probes, activation or lock
//! writes.
use std::{fs, io, path::Path, process::Command};

use turborepo_setup::{
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
}
impl Transports {
    pub fn official(policy: &OfficialSourcePolicy) -> Result<Self, Error> {
        Ok(Self {
            node: NodeTransport::official(policy)?,
            pnpm: PnpmTransport::official(policy)?,
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
    pub node: NodePlan,
    pub pnpm: Option<PnpmPlan>,
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
        .any(|id| !matches!(id.as_str(), "node" | "pnpm"))
    {
        return Err(Error::Unsupported(
            "only managed Node and pnpm are supported",
        ));
    }
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
    let manager = snapshot.package_manager()?;
    let pnpm = if let Some(manager) = manager {
        if manager.manager != Manager::Pnpm {
            return Err(Error::Unsupported("npm override provisioning"));
        }
        Some(PnpmPlan::from_declaration(lock, platform, &node, &manager)?)
    } else {
        None
    };
    Ok(Plans { node, pnpm })
}

pub(super) fn run(
    discovery: &super::root::Discovery,
    snapshot: Snapshot,
    transports: Option<Transports>,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<i32, Error> {
    let root = discovery.snapshot_root()?.as_std_path();
    let lock = snapshot
        .previous_lock()
        .ok_or(Error::Unsupported("frozen mode requires turbo.lock"))?;
    let Plans { node, pnpm } = plans(&snapshot, lock)?;
    let mut desired = vec![node.inventory_tool().clone()];
    if let Some(pnpm) = &pnpm {
        desired.push(pnpm.inventory_tool().clone());
    }
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
    let (prepared_node, prepared_pnpm) = runtime.block_on(async {
        let node = node
            .prepare_if_needed(&store, &desired, &transports.node)
            .await?;
        preflight()?;
        revalidate(discovery)?;
        snapshot.check_guard(&guard)?;
        let pnpm = match &pnpm {
            Some(plan) => {
                plan.prepare_if_needed(&store, &desired, &transports.pnpm)
                    .await?
            }
            None => None,
        };
        Ok::<_, Error>((node, pnpm))
    })?;
    store.reconcile_checked(
        &desired,
        |tool, destination| {
            if tool == node.inventory_tool() {
                prepared_node
                    .as_ref()
                    .ok_or(turborepo_tool_install::Error::InvalidInventory)?
                    .stage(tool, destination)
            } else {
                prepared_pnpm
                    .as_ref()
                    .ok_or(turborepo_tool_install::Error::InvalidInventory)?
                    .stage(tool, destination)
            }
        },
        || {
            storage(root).map_err(io::Error::other)?;
            preflight().map_err(io::Error::other)?;
            revalidate(discovery).map_err(io::Error::other)?;
            snapshot.check_guard(&guard).map_err(io::Error::other)?;
            Ok(())
        },
    )?;
    for tool in &desired {
        println!(
            "{} {}: {}",
            tool.id,
            tool.version,
            if (tool.id == "node" && prepared_node.is_none())
                || (tool.id == "pnpm" && prepared_pnpm.is_none())
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
