mod filter;

use std::collections::HashSet;

use anyhow::Result;
use filter::PackageInference;
use tracing::warn;
use turborepo_scm::SCM;

use crate::{commands::CommandBase, opts::ScopeOpts, package_graph};

pub fn resolve_packages(
    opts: &ScopeOpts,
    base: &CommandBase,
    pkg_graph: &package_graph::PackageGraph,
    _scm: &SCM,
) -> Result<HashSet<String>> {
    let _pkg_inference = opts.pkg_inference_root.as_ref().map(|pkg_inference_path| {
        PackageInference::calculate(&base.repo_root, &pkg_inference_path, pkg_graph)
    });
    warn!("resolve packages not implemented yet");
    Ok(HashSet::new())
}
