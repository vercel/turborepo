//! Package scope resolution.
//!
//! This module delegates to the `turborepo_scope` crate for all scope
//! resolution logic.

use std::collections::HashMap;

use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
};
use turborepo_scm::SCM;
use turborepo_scope::{GitChangeDetector, ScopeChangeDetector, filter::ResolutionError};
use turborepo_turbo_json::TurboJson;
use turborepo_types::{FilterMode, ScopeOpts};

pub fn change_detector<'a>(
    opts: &'a ScopeOpts,
    turbo_root: &'a AbsoluteSystemPath,
    pkg_graph: &'a PackageGraph,
    scm: &'a SCM,
    root_turbo_json: &'a TurboJson,
) -> Result<ScopeChangeDetector<'a>, ResolutionError> {
    let global_deps = opts
        .global_deps
        .iter()
        .map(String::as_str)
        .chain(root_turbo_json.global_deps.iter().map(String::as_str));
    ScopeChangeDetector::new(turbo_root, scm, pkg_graph, global_deps, vec![])
        .map_err(ResolutionError::GlobalDependenciesGlob)
}

pub fn resolve_packages_with_change_detector<T: GitChangeDetector>(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    change_detector: T,
) -> Result<(HashMap<PackageName, PackageInclusionReason>, FilterMode), ResolutionError> {
    turborepo_scope::resolve_packages_with_change_detector(
        opts,
        turbo_root,
        pkg_graph,
        change_detector,
    )
}
