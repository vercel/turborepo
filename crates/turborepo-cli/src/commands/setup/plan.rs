//! Read-only plans: locked reports, plus proposed first-lock, drift and refresh
//! resolutions. Never a writer, repair, download or execution permission.
use std::{cell::Cell, io};

use turborepo_setup::{
    activation::ActivationPlan,
    lock::{Installation, Platform, Snapshot, Tool, reconcile},
    source_policy::ToolDownloadPolicy,
};

use super::{
    Error, LockMode, SetupRequest, check, dependencies::Dependencies, provision, root::Discovery,
};

/// A missing lock, local declaration drift or `--update-lock` proposes the
/// candidate setup would publish. `--force` only reinstalls locked selections.
pub(super) fn report(
    discovery: &Discovery,
    sources: Snapshot,
    request: &SetupRequest,
    dependencies: &Dependencies,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let proposed = request.update_lock
        || match sources.previous_lock() {
            None => true,
            Some(lock) => {
                request.lock == LockMode::Write && !lock.matches_native(sources.declarations())?
            }
        };
    if request.force || !proposed {
        let tool_store = transports
            .as_ref()
            .and_then(provision::Transports::tool_store);
        return inspect_with(
            discovery,
            sources,
            request.force,
            dependencies,
            tool_store.as_deref(),
            preflight,
        );
    }
    propose(
        discovery,
        sources,
        request,
        dependencies,
        transports,
        preflight,
    )
}

/// The plan's heading names a tools-only plan only when it is one.
fn kind(dependencies: &Dependencies) -> &'static str {
    if dependencies.skipped() {
        "tools-only plan"
    } else {
        "plan"
    }
}

/// A tools-only locked plan, for direct tests.
#[cfg(all(test, unix))]
pub(super) fn inspect(
    discovery: &Discovery,
    sources: Snapshot,
    force: bool,
    tool_store: &std::path::Path,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let dependencies = Dependencies::tools_only();
    inspect_with(
        discovery,
        sources,
        force,
        &dependencies,
        Some(tool_store),
        preflight,
    )
}

fn inspect_with(
    discovery: &Discovery,
    sources: Snapshot,
    force: bool,
    dependencies: &Dependencies,
    tool_store: Option<&std::path::Path>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let context = check::context()?;
    let platform = context.artifact_platform();
    preflight()?;
    let lock = sources.previous_lock().ok_or(Error::Unsupported(
        "--plan --force reports reinstalls of locked selections and requires turbo.lock",
    ))?;
    // Share frozen provisioning's native request, artifact and integrity gates.
    provision::plans(&sources, lock)?;
    provision::storage(discovery.snapshot_root()?.as_std_path())?;
    let root = discovery.snapshot_root()?.as_std_path();
    let plan = ActivationPlan::captured(&sources, context)?;
    let store = super::trust::user_store(tool_store, root)?;
    let recorded = super::trust::TrustStore::new(&store, root)?.recorded_env()?;
    // What run and exec would check: the environment's shims and seals.
    let action = match recorded {
        None => "install (missing installation)",
        Some(env) if &env != plan.selection().key() => {
            "repair (installation does not match turbo.lock)"
        }
        Some(env) => match store.activate(root, &env) {
            Ok(_) => "reuse (healthy installation)",
            Err(
                turborepo_tool_install::Error::NotInstalled(_)
                | turborepo_tool_install::Error::Damaged(_),
            ) => "repair (damaged installation)",
            Err(error) => return Err(error.into()),
        },
    };
    let action = if force { "re-verify (--force)" } else { action };
    let mut lines = vec![format!(
        "Locked {} (turbo.lock unchanged):",
        kind(dependencies)
    )];
    for (id, tool) in lock.tools() {
        describe(&mut lines, id, tool, &platform, action);
    }
    lines.extend(dependencies.plan(root, lock)?);
    recheck(discovery, &sources, &preflight)?;
    lines.push("Tracked changes: none. No downloads or probes run.".into());
    Ok(lines.join("\n"))
}

fn propose(
    discovery: &Discovery,
    sources: Snapshot,
    request: &SetupRequest,
    dependencies: &Dependencies,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<String, Error> {
    let platform = provision::platform()?;
    let root = discovery.snapshot_root()?.as_std_path();
    let tool_store = transports
        .as_ref()
        .and_then(provision::Transports::tool_store);
    provision::resolvable(&sources)?;
    // The same gates as setup's resolution, minus the writer it would take.
    let check = || {
        preflight()?;
        revalidate(discovery)?;
        provision::storage(root)
    };
    check()?;
    let policy = preflight()?;
    let upstream = transports.map(Ok).unwrap_or_else(|| {
        provision::Transports::official(&policy, discovery.policy().mirrors())
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mode = match request.lock {
        LockMode::Frozen => reconcile::Mode::Frozen,
        _ if request.update_lock => reconcile::Mode::Refresh,
        _ => reconcile::Mode::Local,
    };
    let resolved = Cell::new(false);
    let outcome = reconcile::propose(
        &sources,
        mode,
        request.offline,
        |resolution| {
            resolved.set(true);
            // Setup's own resolver, so the proposal is the candidate it would write.
            provision::resolve(&runtime, resolution, &upstream)
        },
        |candidate| {
            provision::plans(&sources, candidate)
                .map(drop)
                .map_err(io::Error::other)
        },
        || check().map_err(io::Error::other),
    )?;
    let provision::Plans {
        node,
        pnpm,
        npm,
        uv,
        python,
        rustup,
        rust,
        artifacts,
        go,
        yarn,
        bun,
    } = provision::plans(&sources, &outcome.lock)?;
    let mut desired: Vec<_> = node
        .as_ref()
        .map(|plan| plan.inventory_tool().clone())
        .into_iter()
        .collect();
    desired.extend(pnpm.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(npm.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(uv.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(python.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(rustup.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(rust.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(artifacts.iter().map(|plan| plan.inventory_tool().clone()));
    desired.extend(go.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(yarn.as_ref().map(|plan| plan.inventory_tool().clone()));
    desired.extend(bun.as_ref().map(|plan| plan.inventory_tool().clone()));
    // Same per-tool reuse as setup's preparation: a sealed tree in the store.
    let store = super::trust::user_store(tool_store.as_deref(), root)?;
    let installed = desired
        .iter()
        .filter_map(|tool| {
            store
                .sealed_tree(tool)
                .map(|tree| tree.map(|_| tool))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let previous = sources.previous_lock();
    let change = match previous {
        None => Some("created"),
        Some(previous) if previous != &outcome.lock => Some("updated"),
        Some(_) => None,
    };
    let kind = kind(dependencies);
    let mut lines = vec![match change {
        Some(change) => format!("Proposed {kind} (turbo.lock would be {change}):"),
        None => format!("Proposed {kind} (turbo.lock unchanged):"),
    }];
    for (id, tool) in outcome.lock.tools() {
        let inventory_id = match &tool.installation {
            Installation::Bundled { owner } => owner,
            _ => id,
        };
        let inventory = desired
            .iter()
            .find(|inventory| &inventory.id == inventory_id);
        let action = if inventory.is_some_and(|inventory| installed.contains(&inventory)) {
            "reuse (healthy installation)"
        } else if installed.is_empty() {
            "install (missing installation)"
        } else {
            "install (not in current installation)"
        };
        describe(&mut lines, id, tool, &platform, action);
    }
    if change.is_some() {
        // Exactly the diff setup prints after writing, minus its commit header.
        lines.push("Proposed turbo.lock changes:".into());
        let diff = provision::lock_report(previous, &outcome.lock);
        lines.extend(diff.lines().skip(1).map(|line| format!("  {line}")));
    }
    lines.extend(dependencies.plan(root, &outcome.lock)?);
    recheck(discovery, &sources, &preflight)?;
    lines.push(format!(
        "Tracked changes: {}. {}",
        match change {
            Some(change) => format!("turbo.lock would be {change}; nothing was written"),
            None => "none".into(),
        },
        if resolved.get() {
            // Package-manager resolution verifies archive bytes to lock them.
            "Resolution read upstream metadata and verified archives; nothing was installed, and \
             no probes run."
        } else {
            "No downloads or probes run."
        }
    ));
    Ok(lines.join("\n"))
}

fn describe(lines: &mut Vec<String>, id: &str, tool: &Tool, platform: &Platform, action: &str) {
    lines.push(format!("{id} {}: {action}", tool.version));
    for declaration in &tool.declarations {
        lines.push(format!(
            "  Declaration: {}{} ({})",
            declaration.file,
            declaration
                .field
                .as_ref()
                .map(|field| format!("#{field}"))
                .unwrap_or_default(),
            declaration.request.as_deref().unwrap_or_default(),
        ));
    }
    if let Installation::Managed { artifacts } = &tool.installation {
        for artifact in artifacts
            .get(platform)
            .or_else(|| artifacts.get(&Platform::Any))
            .into_iter()
            .flat_map(|parts| parts.values())
        {
            lines.push(format!("  Source: {}", artifact.url));
            lines.push(format!("  SHA-256: {}", artifact.sha256));
        }
    }
}

fn revalidate(discovery: &Discovery) -> Result<(), Error> {
    if !discovery.revalidate()? {
        return Err(Error::Unsupported(
            "setup discovery changed during plan; retry turbo setup",
        ));
    }
    Ok(())
}

/// Recheck original discovery, policy and exact source/lock bytes before
/// printing anything. Never take a writer guard or install anything.
fn recheck(
    discovery: &Discovery,
    sources: &Snapshot,
    preflight: &impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<(), Error> {
    preflight()?;
    revalidate(discovery)?;
    sources.ensure_current()?;
    provision::storage(discovery.snapshot_root()?.as_std_path())
}
