use anyhow::Result;
use rustc_hash::FxHashMap;
use turbo_tasks::Vc;
use turbopack_core::chunk::{ModuleId, ModuleIds};

/// Counterpart of `Chunk` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleGroup {}

#[turbo_tasks::function]
pub async fn group_modules(deps: Vc<FxHashMap<ModuleId, ModuleIds>>) -> Result<Vc<ModuleGroup>> {}
