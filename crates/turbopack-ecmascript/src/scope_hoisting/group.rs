use anyhow::Result;
use rustc_hash::{FxHashMap, FxHashSet};
use turbo_tasks::Vc;
use turbopack_core::chunk::{ModuleId, ModuleIds};

/// Counterpart of `Scope` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleScope {}

#[turbo_tasks::function]
pub async fn split_scopes(
    entry: Vc<ModuleId>,
    deps: Vc<FxHashMap<ModuleId, ModuleIds>>,
    lazy: Vc<FxHashSet<(ModuleId, ModuleId)>>,
) -> Result<Vc<ModuleScope>> {
    // If a module is imported only as lazy, it should be in a separate scope
}

pub trait DepGraph {
    fn has_edge(&self, from: ModuleId, to: ModuleId) -> bool;
}
