mod change_detector;
mod filter;
mod simple_glob;
pub mod target_selector;

use std::collections::HashSet;

use filter::{FilterResolver, PackageInference};
use turbopath::AbsoluteSystemPath;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_scm::SCM;

pub use crate::run::scope::filter::ResolutionError;
use crate::{opts::ScopeOpts, turbo_json::TurboJson};

#[tracing::instrument(skip(opts, pkg_graph, scm))]
pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    scm: &SCM,
    root_turbo_json: &TurboJson,
) -> Result<(HashSet<PackageName>, bool), ResolutionError> {
    let pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(turbo_root, pkg_inference_path, pkg_graph)
    });

    FilterResolver::new(
        opts,
        pkg_graph,
        turbo_root,
        pkg_inference,
        scm,
        root_turbo_json,
    )?
    .resolve(&opts.get_filters())
}
