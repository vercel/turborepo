use std::collections::HashSet;

use anyhow::Result;

use crate::{commands::CommandBase, opts::ScopeOpts, run::context};

pub fn resolve_packages(
    _opts: &ScopeOpts,
    _base: &CommandBase,
    _ctx: &context::Context,
) -> Result<HashSet<String>> {
    todo!()
}
