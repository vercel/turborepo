use anyhow::Result;
use turbo_tasks::Vc;
use turbopack_core::source::Source;

use crate::CssModuleAssetType;

#[turbo_tasks::value(shared, serialization = "none")]
pub enum ProcessCssResult {
    Ok {
        #[turbo_tasks(trace_ignore)]
        output_code: String,
    },
    Unparseable,
    NotFound,
}

#[turbo_tasks::function]
pub async fn process_css(
    source: Vc<Box<dyn Source>>,
    ty: CssModuleAssetType,
) -> Result<Vc<ProcessCssResult>> {
}
