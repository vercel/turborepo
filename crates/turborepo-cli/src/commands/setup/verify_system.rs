//! `verify-system` JavaScript tools: lock the installed Node.js and pnpm, then
//! verify them on PATH. Nothing is downloaded, installed, or activated, so the
//! managed platform qualification and source preflight do not apply.

use std::io;

use turborepo_setup::{
    lock::{Snapshot, WriteOutcome, reconcile},
    system_tool,
};

use super::{Error, LockMode, provision, root::Discovery};

const DEPENDENCIES: &str = "Dependencies skipped. No tasks run; verify-system tools are not \
                            managed by turbo setup, so tasks use the verified tools on PATH.";

pub(super) fn provision(
    discovery: &Discovery,
    snapshot: Snapshot,
    lock_mode: LockMode,
    resolution_mode: reconcile::Mode,
    force: bool,
) -> Result<i32, Error> {
    let root = discovery.snapshot_root()?.as_std_path();
    let path = std::env::var_os("PATH");
    let path = path.as_deref();
    if !snapshot.declarations().contains_key("node") {
        return Err(Error::Unsupported("a native Node declaration is required"));
    }
    // Force reinstalls managed tools; verify-system tools are only verified.
    let snapshot = if lock_mode == LockMode::Write && !force {
        let check = || {
            provision::revalidate(discovery)?;
            provision::storage(root)
        };
        check()?;
        let outcome = reconcile::reconcile_checked(
            &snapshot,
            resolution_mode,
            false,
            |request| {
                system_tool::resolve(request, path, root)
                    .map_err(|e| reconcile::Error::Resolution(e.to_string()))
            },
            |candidate| {
                system_tool::verify(candidate, path, root)
                    .map(drop)
                    .map_err(io::Error::other)
            },
            || check().map_err(io::Error::other),
        )?;
        let published = snapshot.after_publication(&outcome.lock)?;
        if outcome.publication == Some(WriteOutcome::Written) {
            println!(
                "{}",
                provision::lock_report(snapshot.previous_lock(), &outcome.lock)
            );
        }
        published
    } else {
        snapshot
    };
    for tool in system_tool::verify_locked(&snapshot, path, root)? {
        println!(
            "{} {}: verified at {} (verify-system)",
            tool.id,
            tool.version,
            tool.path.display()
        );
    }
    println!("{DEPENDENCIES}");
    Ok(0)
}

/// Read-only: verifies the committed selections without resolving or writing.
pub(super) fn check(discovery: &Discovery, snapshot: Snapshot) -> Result<i32, Error> {
    let root = discovery.snapshot_root()?.as_std_path();
    let path = std::env::var_os("PATH");
    let verified = system_tool::verify_locked(&snapshot, path.as_deref(), root)?;
    if !discovery.revalidate()? {
        return Err(Error::Unsupported(
            "setup discovery changed during check; retry turbo setup",
        ));
    }
    for tool in verified {
        println!(
            "{} {}: ready (verify-system at {})",
            tool.id,
            tool.version,
            tool.path.display()
        );
    }
    println!("Dependencies skipped; dependency readiness was not checked. No tasks run.");
    Ok(0)
}
