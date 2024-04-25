use anyhow::Result;
use turbo_tasks::Vc;

/// Counterpart of `Chunk` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleGroup {}

#[turbo_tasks::function]
pub async fn group_modules() -> Result<Vc<ModuleGroup>> {}
