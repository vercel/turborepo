//! Locked read-only reporting, not resolution, repair or execution permission.
use turborepo_setup::{
    activation::{ActivationPlan, Error as ReadinessError},
    lock::{Installation, Platform, Snapshot},
    source_policy::OfficialSourcePolicy,
};

use super::{Error, check, provision, root::Discovery};

pub(super) fn inspect(
    discovery: &Discovery,
    sources: Snapshot,
    force: bool,
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<String, Error> {
    let context = check::context()?;
    let platform = context.artifact_platform();
    preflight()?;
    let lock = sources.previous_lock().ok_or(Error::Unsupported(
        "locked plans require turbo.lock; proposed first-lock resolution plans are not implemented",
    ))?;
    // Share frozen provisioning's native request, artifact and integrity gates.
    provision::plans(&sources, lock)?;
    provision::storage(discovery.snapshot_root()?.as_std_path())?;
    let action = match ActivationPlan::inspect_captured(&sources, context) {
        Ok(_) => "reuse (healthy installation)",
        Err(ReadinessError::MissingInventory) => "install (missing installation)",
        Err(ReadinessError::StaleInventory) => "repair (installation does not match turbo.lock)",
        Err(ReadinessError::DamagedInventory(turborepo_tool_install::Error::DamagedContents)) => {
            "repair (damaged installation)"
        }
        Err(error) => return Err(error.into()),
    };
    let action = if force { "reinstall (--force)" } else { action };
    let mut lines = vec!["Locked tools-only plan (turbo.lock unchanged):".to_owned()];
    for (id, tool) in lock.tools() {
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
                .get(&platform)
                .or_else(|| artifacts.get(&Platform::Any))
                .into_iter()
                .flat_map(|parts| parts.values())
            {
                lines.push(format!("  Source: {}", artifact.url));
                lines.push(format!("  SHA-256: {}", artifact.sha256));
            }
        }
    }
    // Recheck original discovery, policy and exact source/lock bytes before
    // printing anything. Never take a writer guard, open Store or a transport.
    preflight()?;
    if !discovery.revalidate()? {
        return Err(Error::Unsupported(
            "setup discovery changed during plan; retry turbo setup",
        ));
    }
    sources.ensure_current()?;
    provision::storage(discovery.snapshot_root()?.as_std_path())?;
    lines.push("Dependencies skipped (--tools-only); dependency readiness was not checked.".into());
    lines.push("Tracked changes: none. No downloads, probes, or tasks run.".into());
    Ok(lines.join("\n"))
}
