use anyhow::Result;

use crate::parse::ParseResult;

#[turbo_tasks::value]
pub struct MergedModule {}

/// Works on single *chunk*
#[turbo_tasks::value]
pub struct ModuleMerger {}

#[turbo_tasks::value_impl]
impl ModuleMerger {
    #[turbo_tasks::value]
    pub async fn merge_modules(&mut self, modules: Vec<Vc<ParseResult>>) -> Result<Vc> {}
}
