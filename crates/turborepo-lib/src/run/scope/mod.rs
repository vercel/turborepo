mod filter;

use std::collections::HashSet;

use anyhow::Result;
use filter::PackageInference;
use tracing::warn;
use turbopath::AbsoluteSystemPath;
use turborepo_scm::SCM;

use crate::{opts::ScopeOpts, package_graph::PackageGraph};

pub fn resolve_packages(
    opts: &ScopeOpts,
    turbo_root: &AbsoluteSystemPath,
    pkg_graph: &PackageGraph,
    _scm: &SCM,
) -> Result<HashSet<String>> {
    let _pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(turbo_root, pkg_inference_path, pkg_graph)
    });
    warn!("resolve packages not implemented yet");
    Ok(HashSet::new())
}
