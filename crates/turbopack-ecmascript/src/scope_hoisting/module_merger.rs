use anyhow::Result;
use turbo_tasks::Vc;

use crate::parse::ParseResult;

#[turbo_tasks::value]
pub struct MergedModule {}

/// Works on single *chunk*
#[turbo_tasks::value]
pub struct ModuleMerger {}

pub async fn merge(modules: Vc<Vec<Vc<ParseResult>>>) {}
