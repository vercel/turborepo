//! Clone-local selection; never a committed lock. Like locked setup, tools
//! install into the shared store and the repository's user record names the
//! environment, holding the selection in place of turbo.lock.
use std::{io, path::Path};

use turborepo_setup::{
    Preparation, ephemeral_reconcile,
    lock::{Installation, Snapshot, reconcile},
    native_baseline::NativeRecord,
    source_policy::ToolDownloadPolicy,
};
use turborepo_tool_install::{Reuse, Selection};

use super::{
    Error,
    output::{Note, ToolOutcome, ToolReport, ToolRow},
    provision, root,
    trust::TrustStore,
};

pub(super) fn run(
    discovery: &root::Discovery,
    snapshot: Snapshot,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<ToolReport, Error> {
    let platform = provision::platform()?;
    // Name what is missing or unsupported instead of the record's generic
    // rejection; both fail before transports, writers, or the store.
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
    // before transports, WRITER, the store, or any ignored-state mutation.
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
    let fixture = transports
        .as_ref()
        .and_then(provision::Transports::tool_store);
    let trust = TrustStore::new(&super::trust::user_store(fixture.as_deref(), root)?, root)?;
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
    let session = trust.store().begin_setup(root, Reuse::Sealed)?;
    let checked = || {
        check().map_err(io::Error::other)?;
        snapshot.check_guard(&guard).map_err(io::Error::other)
    };
    checked()?; // Both acquisitions may have waited; no traffic before this.
    let previous = trust.clone_local_selection()?;
    // An installation from a since-deleted turbo.lock has no clone-local
    // pins to keep; re-resolving would silently move its tools.
    if existing.is_none() && previous.is_none() && trust.recorded_env()?.is_some() {
        return Err(ephemeral_reconcile::Error::Unsupported.into());
    }
    let staged = if existing.is_none() {
        Some(ephemeral_reconcile::stage(
            &snapshot,
            previous.as_deref(),
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
    let (prepared_node, prepared_manager) = runtime.block_on(async {
        let node = node
            .prepare(&session, &desired, &sources.node, Preparation::IfNeeded)
            .await?;
        checked()?;
        if let Some(staged) = &staged {
            staged.check(&snapshot)?;
        }
        // npm and pnpm share the exact, integrity-verified registry transport.
        let manager = match (&pnpm, &npm) {
            (Some(plan), _) => {
                plan.prepare(&session, &desired, &sources.pnpm, Preparation::IfNeeded)
                    .await?
            }
            (_, Some(plan)) => {
                plan.prepare(&session, &desired, &sources.pnpm, Preparation::IfNeeded)
                    .await?
            }
            _ => None,
        };
        Ok::<_, Error>((node, manager))
    })?;
    checked()?;
    let stage_tool = |tool: &turborepo_tool_install::Tool, tree: &Path| {
        let prepared = if tool == node.inventory_tool() {
            prepared_node.as_ref().map(|node| node.stage(tool, tree))
        } else {
            prepared_manager
                .as_ref()
                .map(|manager| manager.stage(tool, tree))
        };
        prepared.ok_or(turborepo_tool_install::Error::InvalidInventory)??;
        if let Some(staged) = &staged {
            staged.verify(tool, tree)?;
        }
        Ok(())
    };
    let recorded = match (&staged, &existing) {
        (Some(staged), _) => staged.selection_bytes(),
        (_, Some(native)) => native.bytes(),
        _ => return Err(Error::Unsupported("missing native record")),
    };
    if let Some(staged) = &staged {
        staged.check(&snapshot)?;
    }
    // Still under the writer guard: the record names the environment, and
    // without turbo.lock, holds the clone-local selection.
    let ensured = session.ensure(&Selection::new(desired.clone())?, stage_tool, |env| {
        checked()?;
        Ok(trust.publish(env, Some(recorded))?)
    })?;
    let mut report = ToolReport {
        downloaded_bytes: prepared_node
            .as_ref()
            .map_or(0, |node| node.downloaded_bytes())
            + prepared_manager
                .as_ref()
                .map_or(0, |manager| manager.downloaded_bytes()),
        ..Default::default()
    };
    for tool in &desired {
        let reused = !ensured.installed.contains(&tool.id);
        report.rows.push(ToolRow {
            id: tool.id.clone(),
            version: tool.version.clone(),
            outcome: if reused {
                ToolOutcome::Reused
            } else {
                ToolOutcome::Installed
            },
            source: None,
        });
    }
    if let Some(npm) = bundled_npm {
        report.rows.push(ToolRow {
            id: "npm".into(),
            version: npm,
            outcome: ToolOutcome::BundledWithNode,
            source: None,
        });
    }
    report.notes.push(Note::NoLock);
    Ok(report)
}
