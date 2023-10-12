use std::collections::HashSet;

use anyhow::Result;
use indexmap::IndexSet;
use once_cell::unsync::Lazy;
use turbo_tasks::{TryFlatJoinIterExt, TryJoinIterExt, Value, Vc};

use super::{
    availability_info::AvailabilityInfo, available_chunk_items::AvailableChunkItemInfo,
    chunk_content, chunking::make_chunks, Chunk, ChunkContentResult, ChunkItem, ChunkingContext,
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
        mut chunk_items,
        async_modules,
        mut external_module_references,
        local_back_edges_inherit_async,
        available_async_modules_back_edges_inherit_async,
    } = chunk_content(chunking_context, entries, Value::new(availability_info)).await?;

    // Find all local chunk items that are self async
    let self_async_children = chunk_items
        .iter()
        .copied()
        .map(|chunk_item| async move {
            let is_self_async = *chunk_item.is_self_async().await?;
            Ok(is_self_async.then_some(chunk_item))
        })
        .try_flat_join()
        .await?;

    // Get all available async modules and concatenate with local async modules
    let mut async_chunk_items = available_async_modules_back_edges_inherit_async
        .keys()
        .copied()
        .chain(self_async_children.into_iter())
        .collect::<IndexSet<_>>();

    // Propagate async inheritance
    let mut i = 0;
    loop {
        let Some(&chunk_item) = async_chunk_items.get_index(i) else {
            break;
        };
        // The first few entries are from
        // available_async_modules_back_edges_inherit_async and need to use that map,
        // all other entries are local
        let map = if i < available_async_modules_back_edges_inherit_async.len() {
            &available_async_modules_back_edges_inherit_async
        } else {
            &local_back_edges_inherit_async
        };
        if let Some(parents) = map.get(&chunk_item) {
            for &parent in parents.iter() {
                // Add item, it will be iterated by this loop too
                async_chunk_items.insert(parent);
            }
        }
        i += 1;
    }

    // If necessary, compute new [AvailabilityInfo]
    let inner_availability_info = Lazy::new(|| {
        let map = chunk_items
            .iter()
            .map(|&chunk_item| {
                (
                    chunk_item,
                    AvailableChunkItemInfo {
                        is_async: async_chunk_items.contains(&chunk_item),
                    },
                )
            })
            .collect();
        let map = Vc::cell(map);
        availability_info.with_chunk_items(map)
    });

    let async_loaders = async_modules
        .into_iter()
        .map(|module| {
            let loader = chunking_context
                .async_loader_chunk_item(module, Value::new(*inner_availability_info));
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
