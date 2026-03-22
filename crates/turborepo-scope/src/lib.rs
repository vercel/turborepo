//! Package scope resolution for Turborepo.
//!
//! This crate handles filtering and selecting packages based on:
//! - Filter patterns (--filter)
//! - Change detection (--affected)
//! - Glob matching
//!
//! Extracted from turborepo-lib to reduce coupling.

#![deny(clippy::all)]
// Allow large error types - ResolutionError contains ChangeMapError which is 128+ bytes.
// Boxing would complicate error handling without significant benefit for a CLI tool.
#![allow(clippy::result_large_err)]

// Module declarations
mod change_detector;
pub mod filter;
pub mod simple_glob;
pub mod target_selector;

use std::collections::HashMap;

pub use change_detector::{GitChangeDetector, ScopeChangeDetector};
pub use filter::{FilterResolver, PackageInference, ResolutionError};
pub use target_selector::{GitRange, InvalidSelectorError, TargetSelector};
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
};
use turborepo_scm::SCM;
pub use turborepo_types::{FilterMode, ScopeOpts};

/// Resolve which packages should be included in the run based on scope options.
///
/// Returns the filtered package set alongside a [`FilterMode`] that
/// describes how the filter was classified (all packages, exclude-only,
/// or explicit selection). The caller uses `FilterMode` to decide
/// whether root tasks should be injected.
#[tracing::instrument(skip(opts, pkg_graph, scm))]
pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    scm: &SCM,
    global_deps: &[String],
) -> Result<(HashMap<PackageName, PackageInclusionReason>, FilterMode), ResolutionError> {
    let pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(turbo_root, pkg_inference_path, pkg_graph)
    });

    FilterResolver::new(opts, pkg_graph, turbo_root, pkg_inference, scm, global_deps)?
        .resolve(&opts.affected_range, opts.get_filters())
}
