//! Healthy locked offline repeat, not download-cache repair or activation.
//! Repository inventory alone can never authorize reuse. No writer, Store
//! guard, resolver, transport construction or executable probe is permitted
//! here.
use turborepo_setup::{
    lock::{Installation, Platform, Snapshot},
    source_policy::OfficialSourcePolicy,
};

use super::{Error as SetupError, authorization::Authorization, check, provision, root::Discovery};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "offline setup cannot reuse the locked installation: {reason}\nRequired artifacts not \
         verified for offline reuse/repair:\n{artifacts}\nRun turbo setup --tools-only without \
         --offline to verify or repair them."
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

/// Whether verified cached bytes (#14540) may still provide what trusted reuse
/// could not authorize. The cache path needs the same safe storage and user
/// state; without them it fails the same way, so this module's report (the
/// cause plus every required artifact) is final.
pub(super) fn cache_may_repair(
    discovery: &Discovery,
    transports: Option<&provision::Transports>,
) -> Result<bool, SetupError> {
    let root = discovery.snapshot_root()?.as_std_path();
    if provision::storage(root).is_err() {
        return Ok(false);
    }
    let base = match transports {
        Some(transports) => transports.trust_base().ok(),
        None => dirs_next::data_local_dir(),
    };
    Ok(base.is_some_and(|base| {
        super::trust::TrustStore::new(&base, root)
            .and_then(|trust| trust.preflight())
            .is_ok()
    }))
}

/// The number of reused tools; the caller reports dependencies.
pub(super) fn run(
    discovery: &Discovery,
    sources: &Snapshot,
    transports: Option<&provision::Transports>,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, SetupError>,
    force: bool,
) -> Result<usize, SetupError> {
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
    // seam. Production always uses the caller's user data directory, never a
    // repository-authored state path. Do NOT construct official transports.
    let base = transports
        .map(provision::Transports::trust_base)
        .transpose()?
        .or_else(dirs_next::data_local_dir)
        .ok_or_else(|| std::io::Error::other("cannot locate setup user state directory"))?;
    let authorization = Authorization::inspect(root, &base, context)
        .map_err(|error| unavailable(error, &artifacts))?;
    // Keep the original discovery/source/lock expectations even though the
    // authorization consumer captures its own snapshot. Recheck them BEFORE the
    // final fresh external-evidence check, not after it or inside its callback.
    inputs()?;
    storage()?;
    let lines = authorization
        .with_current(|plan| {
            plan.tools()
                .iter()
                .map(|tool| {
                    // Offline, the verified installed bytes are the only
                    // reinstall source, and they were just fully re-verified.
                    let outcome = if force {
                        "re-verified (trusted installed bytes; offline --force)"
                    } else {
                        "reused (trusted installed bytes; offline)"
                    };
                    format!("{} {}: {outcome}", tool.id, tool.version)
                })
                .collect::<Vec<_>>()
        })
        .map_err(|error| unavailable(error, &artifacts))?;
    println!("{}", lines.join("\n"));
    Ok(lines.len())
}
