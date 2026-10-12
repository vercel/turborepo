//! Clone-local selection; never a committed lock. Like locked setup, the
//! generation becomes trusted only through the external user-state record.
use std::{io, path::Path};

use turborepo_setup::{
    Preparation, ephemeral_reconcile,
    lock::{Snapshot, reconcile},
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
) -> Result<i32, Error> {
    let platform = provision::platform()?;
    // Name what is missing or unsupported instead of the record's generic
    // rejection; both fail before transports, writers, or Store.
    let declared = snapshot.declarations();
    if !declared.contains_key("node") {
        return Err(Error::NoLockNode);
    }
    if let Some(id) = declared
        .keys()
        .find(|id| !matches!(id.as_str(), "node" | "pnpm"))
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
        provision::storage(root)?;
        snapshot.ensure_current()?;
        Ok::<_, Error>(())
    };
    check()?;
    let policy = preflight()?;
    let sources = transports
        .map(Ok)
        .unwrap_or_else(|| provision::Transports::official(&policy))?;
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
                        &sources.bun,
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
    let provision::Plans {
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
    } = provision::plans(&snapshot, selection)?;
    if npm.is_some() {
        return Err(Error::Unsupported("npm provisioning with --no-lock"));
    }
    if yarn.is_some() || bun.is_some() {
        return Err(Error::Unsupported(
            "Yarn or Bun provisioning with --no-lock",
        ));
    }
    if !artifacts.is_empty() {
        return Err(Error::Unsupported(
            "additional tool provisioning with --no-lock",
        ));
    }
    // The native baseline covers Node and pnpm only.
    if rustup.is_some() || rust.is_some() || uv.is_some() || python.is_some() || go.is_some() {
        return Err(Error::Unsupported(
            "Rust, uv, Python or Go provisioning with --no-lock",
        ));
    }
    let node = node.ok_or(Error::Unsupported("a native Node declaration is required"))?;
    let mut desired = vec![node.inventory_tool().clone()];
    if let Some(pnpm) = &pnpm {
        desired.push(pnpm.inventory_tool().clone());
    }
    let expected = store.generation().or_else(|_| store.repair_generation())?;
    let (prepared_node, prepared_pnpm) = runtime.block_on(async {
        let node = node
            .prepare(&store, &desired, &sources.node, preparation)
            .await?;
        checked()?;
        if let Some(staged) = &staged {
            staged.check(&snapshot, &store)?;
        }
        let pnpm = match &pnpm {
            Some(plan) => {
                plan.prepare(&store, &desired, &sources.pnpm, preparation)
                    .await?
            }
            None => None,
        };
        Ok::<_, Error>((node, pnpm))
    })?;
    checked()?;
    let stage_tool = |tool: &turborepo_tool_install::Tool, tree: &Path| {
        if tool == node.inventory_tool() {
            prepared_node
                .as_ref()
                .ok_or(turborepo_tool_install::Error::InvalidInventory)?
                .stage(tool, tree)
        } else {
            prepared_pnpm
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
        "No turbo.lock written (--no-lock). Clone-local selections do not establish cross-machine \
         reproducibility or frozen readiness."
    );
    println!(
        "Dependencies skipped (--tools-only). No tasks run; managed activation is not enabled."
    );
    Ok(0)
}
