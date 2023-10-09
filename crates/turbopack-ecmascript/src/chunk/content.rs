use turbo_tasks::Vc;
use turbopack_core::{module::Module, output::OutputAsset};

use super::item::EcmascriptChunkItem;

#[turbo_tasks::value(shared)]
pub struct EcmascriptChunkContent {
    pub chunk_items: Vec<Vc<Box<dyn EcmascriptChunkItem>>>,
    pub referenced_output_assets: Vec<Vc<Box<dyn OutputAsset>>>,
    // TODO this need to be removed, instead each chunk item should have async module info attached
    pub chunk_group_root: Option<Vc<Box<dyn Module>>>,
}
