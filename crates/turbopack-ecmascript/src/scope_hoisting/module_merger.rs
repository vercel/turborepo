use anyhow::Result;
use turbo_tasks::Vc;

use crate::parse::ParseResult;

#[turbo_tasks::value]
pub struct MergedModule {}

/// Works on single *chunk*
#[turbo_tasks::value]
pub struct ModuleMerger {}

#[turbo_tasks::value_impl]
impl ModuleMerger {
    #[turbo_tasks::function]
    pub async fn merge_modules(&self, modules: Vec<Vc<ParseResult>>) -> Result<Vc> {}
}
