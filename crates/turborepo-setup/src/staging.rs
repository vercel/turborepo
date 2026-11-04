//! Typed staging inputs adapters share. Each adapter passes what its own
//! staging reads to [`turborepo_tool_install::Staging::new`] with its own
//! revision, so its trees' keys change whenever those inputs do.

use serde::Serialize;

use crate::lock::Format;

/// The entries of an archive in `format` below `root`, copied as they are.
/// An empty `root` names the archive's top level.
#[derive(Serialize)]
pub(crate) struct Archive<'a> {
    pub format: Format,
    pub root: &'a str,
}
