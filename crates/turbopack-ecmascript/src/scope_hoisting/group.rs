use anyhow::Result;
use rustc_hash::{FxHashMap, FxHashSet};
use turbo_tasks::Vc;
use turbopack_core::{
    chunk::{ModuleId, ModuleIds},
    ident::AssetIdent,
    module::Module,
};

/// Counterpart of `Chunk` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleScopeGroup {
    pub scopes: Vc<Vec<Vc<ModuleScope>>>,
}

/// Counterpart of `Scope` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleScope {
    /// The modules in this scope.
    pub modules: Vc<Vec<Vc<Box<dyn Module>>>>,
}

#[turbo_tasks::function]
pub async fn split_scopes(
    entry: Vc<ModuleId>,
    deps: Vc<FxHashMap<ModuleId, ModuleIds>>,
    lazy: Vc<FxHashSet<(ModuleId, ModuleId)>>,
) -> Result<Vc<Vec<Vc<ModuleScopeGroup>>>> {
    // If a module is imported only as lazy, it should be in a separate scope
}

pub trait DepGraph {
    fn get_edge(&self, from: ModuleId, to: ModuleId) -> Option<EdgeData>;
}

#[derive(Debug, Clone, Copy)]
pub struct EdgeData {
    pub is_lazy: bool,
}
