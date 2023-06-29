mod filter;

use std::collections::HashSet;

use anyhow::Result;
use filter::{PackageInference, Resolver};
use tracing::warn;
use turbopath::AbsoluteSystemPath;
use turborepo_scm::SCM;

use crate::{opts::ScopeOpts, package_graph::PackageGraph};

pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    scm: &SCM,
) -> Result<HashSet<String>> {
    let pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(turbo_root, pkg_inference_path, pkg_graph)
    });
    let _resolver = Resolver::new(pkg_graph, turbo_root, pkg_inference, scm);
    let mut filter_patterns = opts.filter_patterns.clone();
    filter_patterns.extend(opts.legacy_filter.as_filter_pattern());
    let is_all_packages = filter_patterns.is_empty() && opts.pkg_inference_root.is_none();
    warn!("resolve packages not implemented yet");
    Ok(HashSet::new())
}
