//! Readiness only. Repo-writable inventory is NOT execution authorization.
use turborepo_setup::{
    activation::ActivationPlan,
    execution_identity::{ExecutionContext, Libc},
    lock::{Platform, Snapshot},
    source_policy::OfficialSourcePolicy,
};

use super::{Error, root::Discovery};

fn context() -> Result<ExecutionContext, Error> {
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
    preflight: impl Fn() -> Result<OfficialSourcePolicy, Error>,
) -> Result<i32, Error> {
    let context = context()?;
    preflight()?;
    let plan = ActivationPlan::inspect(discovery.snapshot_root()?.as_std_path(), context)?;
    // Recheck policy as well as original invocation/root and source bytes. Do
    // not recapture discovery after drift, acquire locks, or initialize Store.
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
    println!(
        "Dependencies skipped (--tools-only); dependency readiness was not checked. No tasks run."
    );
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
