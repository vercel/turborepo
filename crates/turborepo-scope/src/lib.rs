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
mod simple_glob;
pub mod target_selector;

use std::collections::HashMap;

pub use change_detector::{GitChangeDetector, ScopeChangeDetector};
pub use filter::{FilterResolver, PackageInference, ResolutionError};
pub use target_selector::{GitRange, InvalidSelectorError, TargetSelector};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
};
use turborepo_scm::SCM;

/// Options for scope resolution.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScopeOpts {
    /// Root for package inference (from cwd)
    pub pkg_inference_root: Option<AnchoredSystemPathBuf>,
    /// Global dependencies that affect all packages
    pub global_deps: Vec<String>,
    /// Filter patterns from --filter flag
    pub filter_patterns: Vec<String>,
    /// Git range for affected detection (from_ref, to_ref)
    pub affected_range: Option<(Option<String>, Option<String>)>,
}

impl ScopeOpts {
    /// Get the filter patterns.
    pub fn get_filters(&self) -> Vec<String> {
        self.filter_patterns.clone()
    }
}

/// Resolve which packages should be included in the run based on scope options.
///
/// # Arguments
/// * `opts` - Scope resolution options
/// * `turbo_root` - The root of the turbo repository
/// * `pkg_graph` - The package graph
/// * `scm` - Source control manager for change detection
/// * `global_deps` - Global dependencies from turbo.json
///
/// # Returns
/// Tuple of (packages with inclusion reasons, is_all_packages flag)
#[tracing::instrument(skip(opts, pkg_graph, scm))]
pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    scm: &SCM,
    global_deps: &[String],
) -> Result<(HashMap<PackageName, PackageInclusionReason>, bool), ResolutionError> {
    let pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(turbo_root, pkg_inference_path, pkg_graph)
    });

    FilterResolver::new(opts, pkg_graph, turbo_root, pkg_inference, scm, global_deps)?
        .resolve(&opts.affected_range, &opts.get_filters())
}
