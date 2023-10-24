mod change_detector;
mod error;
mod filter;
mod simple_glob;
mod target_selector;

use std::collections::HashSet;

use filter::{FilterResolver, PackageInference};
use turbopath::AbsoluteSystemPath;
use turborepo_scm::SCM;

pub use crate::run::scope::filter::ResolutionError;
use crate::{
    opts::ScopeOpts,
    package_graph::{PackageGraph, WorkspaceName},
};

#[tracing::instrument(skip(opts, pkg_graph, scm))]
pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    scm: &SCM,
) -> Result<HashSet<WorkspaceName>, ResolutionError> {
    let pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(turbo_root, pkg_inference_path, pkg_graph)
    });

    let filtered_packages = FilterResolver::new(opts, pkg_graph, turbo_root, pkg_inference, scm)
        .resolve(&opts.get_filters())?;

    Ok(filtered_packages)
}
