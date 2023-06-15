use std::collections::HashSet;

use anyhow::Result;
use tracing::warn;

use crate::{commands::CommandBase, opts::ScopeOpts, package_graph};

pub fn resolve_packages(
    _opts: &ScopeOpts,
    _base: &CommandBase,
    _ctx: &package_graph::PackageGraph,
) -> Result<HashSet<String>> {
    warn!("resolve packages not implemented yet");
    Ok(HashSet::new())
}
