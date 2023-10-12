use std::collections::HashSet;

use anyhow::Result;
use turbo_tasks::{TryJoinIterExt, Value, Vc};

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

    let async_loaders = async_modules
        .into_iter()
        .map(|module| {
            let loader = chunking_context
                .async_loader_chunk_item(module, Value::new(inner_availability_info));
            loader
        })
        .collect::<Vec<_>>();
    chunk_items.extend(async_loaders.iter().copied());
    let async_loader_references = async_loaders
        .into_iter()
        .map(|loader| loader.references())
        .try_join()
        .await?;
    for references in async_loader_references {
        for &reference in references.iter() {
            external_module_references.insert(reference);
        }
    }

    let output_assets = external_module_references
        .into_iter()
        .map(|reference| reference.resolve_reference().primary_output_assets())
        .try_join()
        .await?;
    let mut set = HashSet::new();
    let output_assets = output_assets
        .iter()
        .flatten()
        .copied()
        .filter(|&asset| set.insert(asset))
        .collect::<Vec<_>>();

    let chunks = make_chunks(
        chunking_context,
        chunk_items,
        output_assets,
        chunk_group_root,
    )
    .await?;

    Ok(MakeChunkGroupResult { chunks })
}
