//! Clone-local selection; never a committed lock. Like locked setup, the
//! generation becomes trusted only through the external user-state record.
use std::{io, path::Path};

use turborepo_setup::{
    Preparation, ephemeral_reconcile,
    lock::{Installation, Snapshot, reconcile},
    native_baseline::NativeRecord,
    source_policy::OfficialSourcePolicy,
};
use turborepo_tool_install::Store;

use super::{Error, provision, root, trust::TrustStore};

pub(super) fn run(
    discovery: &root::Discovery,
    snapshot: Snapshot,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<provision::Provisioned, Error> {
    let platform = provision::platform()?;
    // Name what is missing or unsupported instead of the record's generic
    // rejection; both fail before transports, writers, or Store.
    let declared = snapshot.declarations();
    if !declared.contains_key("node") {
        return Err(Error::NoLockNode);
    }
    if let Some(id) = declared
        .keys()
        .find(|id| !matches!(id.as_str(), "node" | "npm" | "pnpm"))
    {
        return Err(Error::NoLockTool(id.clone()));
    }
    NativeRecord::preflight(&snapshot)?;
    // An existing real lock is authoritative: reject drift/unsupported semantics
    // before transports, WRITER, Store, or any ignored-state mutation.
    let existing = snapshot
        .previous_lock()
        .map(|lock| {
            provision::plans(&snapshot, lock)?;
            Ok::<_, Error>(NativeRecord::from_snapshot(&snapshot, lock)?)
        })
        .transpose()?;
    let root = discovery.snapshot_root()?.as_std_path();
    // User state is checked after the read-only diagnostics above, before any
    // writer or traffic.
    let base = match &transports {
        Some(transports) => transports.trust_base()?,
        None => dirs_next::data_local_dir()
            .ok_or_else(|| io::Error::other("cannot locate setup user state directory"))?,
    };
    let trust = TrustStore::new(&base, root)?;
    let check = || {
        trust.preflight()?;
        preflight()?;
        provision::revalidate(discovery)?;
        provision::storage(root, discovery.git())?;
        snapshot.ensure_current()?;
        Ok::<_, Error>(())
    };
    check()?;
    let policy = preflight()?;
    let sources = transports.map(Ok).unwrap_or_else(|| {
        provision::Transports::official(&policy, discovery.policy().mirrors())
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // Fixed order, retained through discovery/policy checks and atomic selection.
    let guard = snapshot.guard()?;
    check()?;
    snapshot.check_guard(&guard)?;
    let mut store = Store::open(root)?;
    let checked = || {
        check().map_err(io::Error::other)?;
        snapshot.check_guard(&guard).map_err(io::Error::other)
    };
    checked()?; // Both acquisitions may have waited; no traffic before this.
    // Repo inventory proves health, never provenance. Without a matching user
    // record the whole cohort is freshly verified before it can be recorded.
    let recorded = trust.allows_reuse(&store)?;
    let preparation = if recorded {
        Preparation::IfNeeded
    } else {
        Preparation::Fresh
    };
    let staged = if existing.is_none() {
        Some(ephemeral_reconcile::stage(
            &snapshot,
            &store,
            platform,
            false,
            |request| {
                checked().map_err(|e| reconcile::Error::Resolution(e.to_string()))?;
                runtime
                    .block_on(turborepo_setup::js_resolution::resolve(
                        request,
                        &sources.node,
                        &sources.registry,
                    ))
                    .map_err(|e| reconcile::Error::Resolution(e.to_string()))
            },
            checked,
        )?)
    } else {
        None
    };
    let selection = match (&staged, &existing) {
        (Some(staged), _) => staged.selection(),
        (_, Some(native)) => native.selection(),
        _ => return Err(Error::Unsupported("missing native selection")),
    };
    // NativeRecord::preflight admits only Node with npm or pnpm; Node-bundled
    // npm adds no tool and stays bound to Node's bytes.
    let provision::Plans {
        node, pnpm, npm, ..
    } = provision::plans(&snapshot, selection)?;
    let bundled_npm = selection
        .tools()
        .get("npm")
        .filter(|npm| matches!(npm.installation, Installation::Bundled { .. }))
        .map(|npm| npm.version.clone());
    let node = node.ok_or(Error::Unsupported("a native Node declaration is required"))?;
    // At most one manager tool: pnpm, or npm unless Node bundles it.
    let mut desired = vec![node.inventory_tool().clone()];
    desired.extend(pnpm.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(npm.as_ref().map(|plan| plan.inventory_tool().clone()));
    let expected = store.generation().or_else(|_| store.repair_generation())?;
    let (prepared_node, prepared_manager) = runtime.block_on(async {
        let node = node
            .prepare(&store, &desired, &sources.node, preparation)
            .await?;
        checked()?;
        if let Some(staged) = &staged {
            staged.check(&snapshot, &store)?;
        }
        // npm and pnpm share the exact, integrity-verified registry transport.
        let manager = match (&pnpm, &npm) {
            (Some(plan), _) => {
                plan.prepare(&store, &desired, &sources.pnpm, preparation)
                    .await?
            }
            (_, Some(plan)) => {
                plan.prepare(&store, &desired, &sources.pnpm, preparation)
                    .await?
            }
            _ => None,
        };
        Ok::<_, Error>((node, manager))
    })?;
    checked()?;
    let stage_tool = |tool: &turborepo_tool_install::Tool, tree: &Path| {
        if tool == node.inventory_tool() {
            prepared_node
                .as_ref()
                .ok_or(turborepo_tool_install::Error::InvalidInventory)?
                .stage(tool, tree)
        } else {
            prepared_manager
                .as_ref()
                .ok_or(turborepo_tool_install::Error::InvalidInventory)?
                .stage(tool, tree)
        }
    };
    if let Some(staged) = staged {
        staged.check(&snapshot, &store)?;
        staged.publish_guarded(
            &snapshot, &mut store, &guard, !recorded, stage_tool, checked,
        )?;
    } else {
        let native = existing.ok_or(Error::Unsupported("missing native record"))?;
        store.reconcile_recorded_checked(
            &desired,
            native.record(),
            &expected,
            !recorded,
            stage_tool,
            || checked().map_err(Into::into),
        )?;
    }
    // Still under both guards: record the selected generation, identified by
    // turbo.lock or, without one, by its sealed clone-local selection.
    checked()?;
    trust.publish(&store)?;
    let mut provisioned = provision::Provisioned {
        downloaded_bytes: prepared_node
            .as_ref()
            .map_or(0, |node| node.downloaded_bytes())
            + prepared_manager
                .as_ref()
                .map_or(0, |manager| manager.downloaded_bytes()),
        ..Default::default()
    };
    for tool in &desired {
        let reused = (tool.id == "node" && prepared_node.is_none())
            || (tool.id != "node" && prepared_manager.is_none());
        if reused {
            provisioned.reused += 1;
        } else {
            provisioned.installed += 1;
        }
        println!(
            "{} {}: {}",
            tool.id,
            tool.version,
            if reused { "reused" } else { "installed" }
        );
    }
    if let Some(npm) = bundled_npm {
        println!("npm {npm}: provided by managed Node (bundled)");
    }
    println!(
        "No turbo.lock written (--no-lock). Clone-local selections do not establish cross-machine \
         reproducibility or frozen readiness."
    );
    println!("turbo run uses these tools in this clone only.");
    Ok(provisioned)
}
