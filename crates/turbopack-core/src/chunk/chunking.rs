use std::mem::take;

use anyhow::Result;
use indexmap::IndexMap;
use turbo_tasks::{TryJoinIterExt, ValueToString, Vc};

use super::{Chunk, ChunkItem, ChunkType, ChunkingContext};
use crate::{ident::AssetIdent, module::Module, output::OutputAsset};

pub async fn make_chunks(
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    ident: Vc<AssetIdent>,
    chunk_items: impl IntoIterator<Item = Vc<Box<dyn ChunkItem>>>,
    mut referenced_output_assets: Vec<Vc<Box<dyn OutputAsset>>>,
    chunk_group_root: Option<Vc<Box<dyn Module>>>,
) -> Result<Vec<Vc<Box<dyn Chunk>>>> {
    let chunk_items = chunk_items
        .into_iter()
        .map(|chunk_item| async move {
            let ty = chunk_item.ty().resolve().await?;
            Ok((ty, chunk_item))
        })
        .try_join()
        .await?;
    let names = chunk_items
        .iter()
        .map(|(_, chunk_item)| chunk_item.asset_ident().to_string())
        .try_join()
        .await?;
    println!(
        "make_chunks(\n  {}\n)",
        names
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",\n  ")
    );
    let mut map = IndexMap::<_, Vec<_>>::new();
    for (ty, chunk_item) in chunk_items {
        map.entry(ty).or_default().push(chunk_item);
    }

    let mut chunks = Vec::new();
    for (ty, chunk_items) in map {
        let chunk_items = Vc::cell(chunk_items);
        chunks.push(ty.chunk(
            chunking_context,
            ident,
            chunk_items,
            Vc::cell(take(&mut referenced_output_assets)),
            chunk_group_root,
        ));
    }

    Ok(chunks)
}
