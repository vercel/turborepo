use anyhow::Result;
use turbo_tasks::{Value, Vc};

use super::{
    availability_info::AvailabilityInfo, chunk_content, chunking::make_chunks, Chunk,
    ChunkContentResult, ChunkItem, ChunkingContext,
};
use crate::{module::Module, reference::ModuleReference};

pub struct MakeChunkGroupResult {
    pub chunks: Vec<Vc<Box<dyn Chunk>>>,
}

/// Creates a chunk group from a set of entries.
pub async fn make_chunk_group(
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    entries: impl IntoIterator<Item = Vc<Box<dyn Module>>>,
    availability_info: AvailabilityInfo,
    chunk_group_root: Option<Vc<Box<dyn Module>>>,
) -> Result<MakeChunkGroupResult> {
    let ChunkContentResult {
        modules,
        mut chunk_items,
        async_modules,
        mut external_module_references,
    } = chunk_content(chunking_context, entries, Value::new(availability_info)).await?;

    let inner_availability_info = availability_info.with_modules(modules);

    for module in async_modules {
        let loader =
            chunking_context.async_loader_chunk_item(module, Value::new(inner_availability_info));
        chunk_items.insert(loader);
        for &reference in loader.references().await?.iter() {
            external_module_references.insert(reference);
        }
    }

    let mut output_assets = Vec::new();
    for reference in external_module_references {
        for &output_asset in reference
            .resolve_reference()
            .primary_output_assets()
            .await?
            .iter()
        {
            output_assets.push(output_asset);
        }
    }

    let chunks = make_chunks(
        chunking_context,
        chunk_items,
        output_assets,
        chunk_group_root,
    )
    .await?;

    Ok(MakeChunkGroupResult { chunks })
}
