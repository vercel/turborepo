//! Trusted read-only offline repeats, with verified-cache repair for Node,
//! registry adapters, and additional tools. Repository inventory alone never
//! authorizes reuse. Cache misses are diagnosed before writers; no resolver or
//! executable probe.
use turborepo_setup::{
    lock::{Installation, Platform, Snapshot},
    source_policy::ToolDownloadPolicy,
};

use super::{
    Error as SetupError,
    authorization::Authorization,
    check,
    output::{Note, ToolOutcome, ToolReport, ToolRow},
    provision,
    root::Discovery,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "offline setup is missing verified cached artifacts; nothing was installed\noffline setup \
         cannot reuse the locked installation: {reason}\nRequired artifacts not verified for \
         offline reuse/repair:\n{artifacts}\nRun turbo setup --tools-only without --offline to \
         download and verify them. No complete verified download-cache source is available."
    )]
    Unavailable {
        #[source]
        reason: Box<dyn std::error::Error + Send + Sync>,
        artifacts: String,
    },
}

fn unavailable(reason: impl std::error::Error + Send + Sync + 'static, artifacts: &str) -> Error {
    Error::Unavailable {
        reason: Box::new(reason),
        artifacts: artifacts.to_owned(),
    }
}

fn inspect(
    discovery: &Discovery,
    sources: &Snapshot,
    transports: Option<&provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, SetupError>,
    force: bool,
) -> Result<Vec<ToolRow>, SetupError> {
    let context = check::context()?;
    let platform = context.artifact_platform();
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
    // Diagnose this before inventory or cache access, preserving the original
    // cache-only provisioning request's actionable failure.
    if !lock.matches_native(sources.declarations())? || !sources.matches_additional(lock) {
        return Err(SetupError::Unsupported(
            "offline setup cannot resolve changed tool declarations; run turbo setup --tools-only \
             without --offline",
        ));
    }
    // Reuse exactly the same native constraints, authored integrity and host
    // mappings as locked provisioning, before looking at any local inventory.
    provision::plans(sources, lock)?;
    let artifacts = lock
        .tools()
        .iter()
        .flat_map(|(id, tool)| {
            let Installation::Managed { artifacts } = &tool.installation else {
                return Vec::new();
            };
            artifacts
                .get(&platform)
                .or_else(|| artifacts.get(&Platform::Any))
                .into_iter()
                .flat_map(|parts| parts.iter())
                .map(|(part, artifact)| {
                    format!(
                        "  {id} {} [{part}]: {} (SHA-256 {})",
                        tool.version, artifact.url, artifact.sha256
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Diagnostics come from the validated lock, not unsafe storage or repo
    // inventory. Retain the storage cause and the complete artifact set.
    let storage = || provision::storage(root).map_err(|error| unavailable(error, &artifacts));
    storage()?;
    // The only fixture override is already private to setup's test transport
    // seam. Production always locates the user's tool store, never a
    // repository-authored state path. Do NOT construct official transports.
    let fixture = transports.and_then(provision::Transports::tool_store);
    let authorization =
        Authorization::verify(root, fixture.as_deref(), context, sources.ecosystems())
            .map_err(|error| unavailable(error, &artifacts))?;
    // Keep the original discovery/source/lock expectations even though the
    // authorization consumer captures its own snapshot. Recheck them BEFORE the
    // final fresh external-evidence check, not after it or inside its callback.
    inputs()?;
    storage()?;
    let rows = authorization
        .with_current(|plan| {
            plan.tools()
                .iter()
                .map(|tool| ToolRow {
                    id: tool.id.clone(),
                    version: tool.version.clone(),
                    // Offline, the verified installed bytes are the only
                    // reinstall source, and they were just fully re-verified.
                    outcome: if force {
                        ToolOutcome::Reverified
                    } else {
                        ToolOutcome::ReusedTrusted
                    },
                    source: None,
                })
                .collect::<Vec<_>>()
        })
        .map_err(|error| unavailable(error, &artifacts))?;
    Ok(rows)
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
    sources: Snapshot,
    request: &super::SetupRequest,
    transports: Option<provision::Transports>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, SetupError>,
) -> Result<ToolReport, SetupError> {
    let checked = inspect(
        discovery,
        &sources,
        transports.as_ref(),
        &preflight,
        request.force,
    );
    if checked.is_ok() && !request.force {
        return checked.map(report);
    }
    // Source, declaration, and policy errors must never become a repair attempt.
    if checked
        .as_ref()
        .err()
        .is_some_and(|error| !matches!(error, SetupError::Offline(_)))
    {
        return checked.map(report);
    }
    let lock = sources
        .previous_lock()
        .ok_or(SetupError::Unsupported("offline setup requires turbo.lock"))?;
    if lock
        .tools()
        .iter()
        .any(|(id, tool)| !provision::offline_cacheable(id, tool))
    {
        // Other adapters still support only the original trusted read-only repeat.
        return checked.map(report);
    }
    let plans = provision::plans(&sources, lock)?;
    let transports = transports.map(Ok).unwrap_or_else(|| {
        provision::Transports::official(&preflight()?, discovery.policy().mirrors())
    })?;
    let available: Vec<_> = [
        plans
            .node
            .as_ref()
            .map(|plan| ("node", plan.cache_available(&transports.node))),
        plans
            .pnpm
            .as_ref()
            .map(|plan| ("pnpm", plan.cache_available(&transports.pnpm))),
        plans
            .npm
            .as_ref()
            .map(|plan| ("npm", plan.cache_available(&transports.pnpm))),
    ]
    .into_iter()
    .flatten()
    .chain(plans.artifacts.iter().map(|plan| {
        (
            plan.inventory_tool().id.as_str(),
            plan.cache_available(transports.node.cache()),
        )
    }))
    .collect();
    if checked.is_err() && available.iter().any(|(_, cached)| !cached) {
        // Retain the original storage/provenance cause when no verified repair
        // source exists; neither miss path opens a writer or user-state store.
        if !available.iter().any(|(_, cached)| *cached) {
            return checked.map(report);
        }
        let missing: Vec<_> = available
            .iter()
            .filter_map(|(id, cached)| (!cached).then_some(*id))
            .collect();
        return Err(SetupError::OfflineMissing(provision::missing_artifacts(
            lock,
            provision::platform()?,
            &missing,
        )));
    }
    // Cache reads above are preflight only. Locked preparation verifies every
    // byte again, and publishes fresh external provenance under both guards.
    provision::run(discovery, sources, request, Some(transports), preflight)
}
