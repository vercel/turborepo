//! Read-only lock diagnostics before transports, writers or installation state.
use std::collections::BTreeSet;

use miette::Diagnostic;
use turborepo_setup::lock::{Lock, Snapshot};

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("frozen mode requires turbo.lock")]
    #[diagnostic(help(
        "Run `turbo setup --tools-only` locally and commit turbo.lock, then retry CI."
    ))]
    Missing,
    #[error("turbo.lock declarations changed: {0}")]
    #[diagnostic(help(
        "Run `turbo setup --tools-only` locally and commit the updated turbo.lock, then retry CI."
    ))]
    Drift(String),
}

pub(super) fn matching(snapshot: &Snapshot, lock: &Lock) -> Result<(), super::Error> {
    if lock.matches_native(snapshot.declarations())? {
        return Ok(());
    }
    let mut changed = BTreeSet::new();
    for id in snapshot.declarations().keys().chain(lock.tools().keys()) {
        let current = snapshot
            .declarations()
            .get(id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let previous = lock
            .tools()
            .get(id)
            .map(|tool| tool.declarations.as_slice())
            .unwrap_or(&[]);
        // Provenance is a set. Include both added and removed locations, so
        // removing a declaration still names the original file/field.
        for declaration in current
            .iter()
            .filter(|d| !previous.contains(d))
            .chain(previous.iter().filter(|d| !current.contains(d)))
        {
            changed.insert(format!(
                "{id}: {}{}",
                declaration.file,
                declaration
                    .field
                    .as_ref()
                    .map(|field| format!(" ({field})"))
                    .unwrap_or_default()
            ));
        }
    }
    Err(Error::Drift(changed.into_iter().collect::<Vec<_>>().join(", ")).into())
}

pub(super) fn frozen(snapshot: &Snapshot) -> Result<(), super::Error> {
    let lock = snapshot.previous_lock().ok_or(Error::Missing)?;
    matching(snapshot, lock)
}
