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
pub use turborepo_scope::{filter::ResolutionError, target_selector, ScopeOpts};

use crate::turbo_json::TurboJson;

/// Resolve which packages should be included in the run based on scope options.
///
/// Delegates directly to `turborepo_scope::resolve_packages`.
#[tracing::instrument(skip(opts, pkg_graph, scm))]
pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    scm: &SCM,
    root_turbo_json: &TurboJson,
) -> Result<(HashMap<PackageName, PackageInclusionReason>, bool), ResolutionError> {
    // Delegate to turborepo-scope, passing global_deps from turbo.json
    turborepo_scope::resolve_packages(
        opts,
        turbo_root,
        pkg_graph,
        scm,
        &root_turbo_json.global_deps,
    )
}
