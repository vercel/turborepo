//! Package scope resolution for Turborepo.
//!
//! This crate handles filtering and selecting packages based on:
//! - Filter patterns (--filter)
//! - Change detection (--affected)
//! - Glob matching
//!
//! Extracted from the former monolithic CLI crate to reduce coupling.

#![deny(clippy::all)]
#![cfg_attr(not(test), deny(clippy::expect_used, clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
// Allow large error types - ResolutionError contains ChangeMapError which is 128+ bytes.
// Boxing would complicate error handling without significant benefit for a CLI tool.
#![allow(clippy::result_large_err)]

// Module declarations
mod change_detector;
pub mod filter;
pub mod simple_glob;
pub mod target_selector;

use std::collections::HashMap;

pub use change_detector::{ChangedFilesDetector, GitChangeDetector, ScopeChangeDetector};
pub use filter::{FilterResolver, PackageInference, ResolutionError};
pub use target_selector::{GitRange, InvalidSelectorError, TargetSelector};
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
};
use turborepo_types::{FilterMode, ScopeOpts};

/// Resolve through the production scope entry point with an injected change
/// detector. Repository discovery and Git observations can both be supplied in
/// memory by contract tests without changing the CLI path above.
#[tracing::instrument(skip(opts, pkg_graph, change_detector))]
pub fn resolve_packages_with_change_detector<T: GitChangeDetector>(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    change_detector: T,
) -> Result<(HashMap<PackageName, PackageInclusionReason>, FilterMode), ResolutionError> {
    let pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(turbo_root, pkg_inference_path, pkg_graph)
    });
    FilterResolver::new_with_change_detector(pkg_graph, turbo_root, pkg_inference, change_detector)
        .resolve(&opts.affected_range, opts.get_filters())
}

#[cfg(test)]
mod entrypoint_tests;
