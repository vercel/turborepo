//! Offline setup: a trusted lockless repeat when this repository's recorded
//! installation is current, otherwise locked provisioning from trees already
//! in the tool store. Repository inventory alone never authorizes reuse. No
//! resolver, download, or executable probe.
use turborepo_setup::source_policy::ToolDownloadPolicy;

use super::{
    Error as SetupError,
    authorization::Authorization,
    check,
    output::{Note, ToolOutcome, ToolReport, ToolRow},
    provision,
    root::Discovery,
};

/// The trusted repeat's rows, or `None` when this repository's recorded
/// installation is missing, stale, or damaged. Source, declaration, policy,
/// and storage errors are errors, never `None`.
fn inspect(
    discovery: &Discovery,
    sources: &turborepo_setup::lock::Snapshot,
    transports: Option<&provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, SetupError>,
    force: bool,
) -> Result<Option<Vec<ToolRow>>, SetupError> {
    let context = check::context()?;
    let root = discovery.snapshot_root()?.as_std_path();
    let inputs = || {
        preflight()?;
        if !discovery.revalidate()? {
            return Err(SetupError::Unsupported(
                "setup discovery changed during offline reuse; retry turbo setup",
            ));
        }
        sources.ensure_current()?;
        Ok::<_, SetupError>(())
    };
    inputs()?;
    let lock = sources.previous_lock().ok_or(SetupError::Unsupported(
        "offline setup requires turbo.lock; run turbo setup --tools-only online and commit the \
         lock",
    ))?;
    // Offline never resolves changed declarations, regardless of lock mode.
    if !lock.matches_native(sources.declarations())? || !sources.matches_additional(lock) {
        return Err(SetupError::Unsupported(
            "offline setup cannot resolve changed tool declarations; run turbo setup --tools-only \
             without --offline",
        ));
    }
    // Reuse exactly the same native constraints, authored integrity and host
    // mappings as locked provisioning, before looking at any local inventory.
    provision::plans(sources, lock)?;
    provision::storage(root)?;
    // The only fixture override is already private to setup's test transport
    // seam. Production always locates the user's tool store, never a
    // repository-authored state path. Do NOT construct official transports.
    let fixture = transports.and_then(provision::Transports::tool_store);
    let authorization =
        match Authorization::inspect(root, fixture.as_deref(), context, sources.ecosystems()) {
            Ok(authorization) => authorization,
            Err(error) => {
                tracing::debug!("offline setup cannot repeat the recorded installation: {error}");
                return Ok(None);
            }
        };
    let store = super::trust::user_store(fixture.as_deref(), root)?;
    // Keep the original discovery/source/lock expectations even though the
    // authorization consumer captures its own snapshot. Recheck them BEFORE the
    // final fresh external-evidence check, not after it or inside its callback.
    inputs()?;
    provision::storage(root)?;
    let checked = authorization.with_current(|plan, _| {
        // Offline, the installed trees are the only reinstall source, so
        // every byte is re-verified against its seal.
        store.check(plan.selection())?;
        Ok::<_, turborepo_tool_install::Error>(
            plan.tools()
                .iter()
                .map(|tool| ToolRow {
                    id: tool.id.clone(),
                    version: tool.version.clone(),
                    outcome: if force {
                        ToolOutcome::Reverified
                    } else {
                        ToolOutcome::ReusedTrusted
                    },
                    source: None,
                })
                .collect::<Vec<_>>(),
        )
    });
    match checked
        .map_err(|error| error.to_string())
        .and_then(|rows| rows.map_err(|error| error.to_string()))
    {
        Ok(rows) => Ok(Some(rows)),
        Err(reason) => {
            tracing::debug!("offline setup cannot repeat the recorded installation: {reason}");
            Ok(None)
        }
    }
}

/// Trusted reuse touches no tools; the caller reports dependencies, which
/// may still install offline from the native caches.
fn report(rows: Vec<ToolRow>) -> ToolReport {
    ToolReport {
        rows,
        notes: vec![Note::TrustedRepeat],
        downloaded_bytes: 0,
    }
}

pub(super) fn run(
    discovery: &Discovery,
    sources: turborepo_setup::lock::Snapshot,
    request: &super::SetupRequest,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, SetupError>,
) -> Result<ToolReport, SetupError> {
    if let Some(rows) = inspect(
        discovery,
        &sources,
        transports.as_ref(),
        &preflight,
        request.force,
    )? {
        return Ok(report(rows));
    }
    // Name every tool the store lacks before taking a lock or writing.
    // Provisioning re-hashes the rest under the store lease.
    let lock = sources
        .previous_lock()
        .ok_or(SetupError::Unsupported("offline setup requires turbo.lock"))?;
    let root = discovery.snapshot_root()?.as_std_path();
    let fixture = transports
        .as_ref()
        .and_then(provision::Transports::tool_store);
    let store = super::trust::user_store(fixture.as_deref(), root)?;
    let tools = provision::plans(&sources, lock)?.tools();
    let mut missing = Vec::new();
    for tool in &tools {
        if store.sealed_tree(tool)?.is_none() {
            missing.push(tool.id.as_str());
        }
    }
    if !missing.is_empty() {
        return Err(SetupError::OfflineMissing(provision::missing_artifacts(
            lock,
            provision::platform()?,
            &missing,
        )));
    }
    provision::run(discovery, sources, request, transports, preflight)
}
