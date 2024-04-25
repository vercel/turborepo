use anyhow::Result;
use rustc_hash::FxHashMap;
use turbo_tasks::Vc;
use turbopack_core::chunk::{ModuleId, ModuleIds};

/// Counterpart of `Scope` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleScope {}

#[turbo_tasks::function]
pub async fn split_scopes(
    entry: Vc<ModuleId>,
    deps: Vc<FxHashMap<ModuleId, ModuleIds>>,
) -> Result<Vc<ModuleScope>> {
    // If a module is imported only as lazy, it should be in a separate scope
}
