//! Readiness only: re-hashes every installed tree the lock selects. This is
//! NOT execution authorization.
use turborepo_setup::{
    activation::ActivationPlan,
    execution_identity::{ExecutionContext, Libc},
    lock::{Platform, Snapshot},
    source_policy::ToolDownloadPolicy,
};

use super::{Error, root::Discovery};

pub(super) fn context() -> Result<ExecutionContext, Error> {
    // Share provisioning's qualification: notably no Windows/musl fallback.
    let platform = super::provision::platform()?;
    let libc = match platform {
        Platform::LinuxX64Gnu | Platform::LinuxArm64Gnu => Libc::Gnu { abi: "gnu".into() },
        _ => Libc::None,
    };
    // These labels are for read-only readiness, not runtime ABI/CPU probing or
    // a task-cache identity. Nothing here may authorize or launch a process.
    ExecutionContext::new(
        turborepo_platform::Platform::current(),
        libc,
        "host-readiness".into(),
        ["unprobed".into()].into(),
    )
    .map_err(|_| Error::Unsupported("cannot select setup check host platform"))
}

pub(super) fn run(
    discovery: &Discovery,
    sources: Snapshot,
    dependencies: &super::dependencies::Dependencies,
    tool_store: Option<&std::path::Path>,
    preflight: impl Fn() -> Result<ToolDownloadPolicy, Error>,
) -> Result<i32, Error> {
    let context = context()?;
    preflight()?;
    // Keep the original captured sources and their workspace-flag gating.
    let plan = ActivationPlan::captured(&sources, context)?;
    let root = discovery.snapshot_root()?.as_std_path();
    super::trust::user_store(tool_store, root)?.check(plan.selection())?;
    // Recheck policy as well as original invocation/root and source bytes. Do
    // not recapture discovery after drift or acquire locks.
    preflight()?;
    if !discovery.revalidate()? {
        return Err(Error::Unsupported(
            "setup discovery changed during check; retry turbo setup",
        ));
    }
    sources.ensure_current()?;
    for tool in plan.tools() {
        println!("{} {}: ready", tool.id, tool.version);
    }
    if let Some(npm) = sources
        .previous_lock()
        .and_then(|lock| lock.tools().get("npm"))
        && matches!(
            npm.installation,
            turborepo_setup::lock::Installation::Bundled { .. }
        )
    {
        println!("npm {}: ready (bundled with managed Node)", npm.version);
    }
    // The activation plan above already required turbo.lock. Dependency
    // readiness is never claimed.
    if let Some(lock) = sources.previous_lock() {
        for line in dependencies.check(root, lock)? {
            println!("{line}");
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_context_selects_the_actual_qualified_host() {
        match super::super::provision::platform() {
            Ok(platform) => assert_eq!(context().unwrap().artifact_platform(), platform),
            Err(_) => assert!(context().is_err()),
        }
    }
}
