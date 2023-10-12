use turbo_tasks::Vc;
use turbopack_core::{chunk::AsyncModuleInfo, output::OutputAsset};

use super::item::EcmascriptChunkItem;

#[turbo_tasks::value(shared)]
pub struct EcmascriptChunkContent {
    pub chunk_items: Vec<(
        Vc<Box<dyn EcmascriptChunkItem>>,
        Option<Vc<AsyncModuleInfo>>,
    )>,
    pub referenced_output_assets: Vec<Vc<Box<dyn OutputAsset>>>,
}
