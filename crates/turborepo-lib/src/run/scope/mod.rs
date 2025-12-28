//! Package scope resolution.
//!
//! This module delegates to the `turborepo_scope` crate for all scope
//! resolution logic. Re-exports are provided for backward compatibility.

use std::collections::HashMap;

use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
};
use turborepo_scm::SCM;
// Re-export modules and types from turborepo-scope crate for backward compatibility
pub use turborepo_scope::filter;
pub use turborepo_scope::{filter::ResolutionError, target_selector};

use crate::{opts::ScopeOpts, turbo_json::TurboJson};

/// Resolve which packages should be included in the run based on scope options.
///
/// This function converts from turborepo-lib's `ScopeOpts` to turborepo-scope's
/// `ScopeOpts` and delegates to `turborepo_scope::resolve_packages`.
#[tracing::instrument(skip(opts, pkg_graph, scm))]
pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    scm: &SCM,
    root_turbo_json: &TurboJson,
) -> Result<(HashMap<PackageName, PackageInclusionReason>, bool), ResolutionError> {
    // Convert turborepo-lib ScopeOpts to turborepo-scope ScopeOpts
    let scope_opts = turborepo_scope::ScopeOpts {
        pkg_inference_root: opts.pkg_inference_root.clone(),
        global_deps: opts.global_deps.clone(),
        filter_patterns: opts.filter_patterns.clone(),
        affected_range: opts.affected_range.clone(),
    };

    // Delegate to turborepo-scope, passing global_deps from turbo.json
    turborepo_scope::resolve_packages(
        &scope_opts,
        turbo_root,
        pkg_graph,
        scm,
        &root_turbo_json.global_deps,
    )
}
