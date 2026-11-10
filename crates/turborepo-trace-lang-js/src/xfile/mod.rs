//! Cross-file parameter binding for JS / TS (graph mode).
//!
//! The value-flow engine ([`crate::flow`]) stops at the file boundary: a file
//! read or module load whose argument depends on a parameter of an exported
//! function gets an uncertainty record with origin `Param`, and every entry
//! that reaches the file is selected on every change. The dependency really
//! belongs to the callers that supply the value. This module moves it there:
//!
//! 1. [`facts`]: per-file facts, a pure function of the file's text
//!    ([`file_facts`]). The sites that depend on parameters as expressions over
//!    *holes*, the file's imports, re-exports and exports, and every call of an
//!    imported binding with the values of its arguments.
//! 2. [`bind`]: given the facts of the files involved and the import edges of
//!    the graph, every hole is bound to the arguments of the calls that can
//!    reach it, across files, through re-export barrels, and transitively
//!    through the callers' own parameters. The result is a set of bound
//!    references on the files that *supply* the values, and the set of sites
//!    whose callers are all known (a closed world), whose records can be
//!    dropped.
//!
//! Nothing here touches the file system or the graph store: the caller provides
//! both through [`bind::World`], and turns the result into graph facts.
//!
//! Binding needs every importer of a module, so it exists in graph mode only.
//! A single trace follows files from its entries and never sees the other
//! callers; it keeps the records.

pub mod bind;
pub mod codec;
pub mod facts;
pub mod show;
#[cfg(test)]
mod tests;
pub mod trace;

use std::path::Path;

pub use facts::FileFacts;

use crate::{
    extract::{ScanOptions, scan_named},
    lexer::Dialect,
};

/// The cross-file facts of the file at `path` with contents `src`, or `None`
/// when the file cannot be analyzed (it does not parse, is too large, or uses
/// `eval` / `with`). A pure function of `src` and the file's name.
pub fn file_facts(src: &[u8], path: &Path) -> Option<FileFacts> {
    let dialect = Dialect::from_path(path);
    let file_name = path.file_name().and_then(|n| n.to_str());
    let ex = scan_named(src, dialect, ScanOptions::default(), file_name);
    crate::flow::facts::extract(src, dialect, file_name, &ex.flow_sites)
}
