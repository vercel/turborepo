use std::{
    mem::{replace, take},
    pin::Pin,
};

use anyhow::Result;
use futures::Future;
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;
use tracing::Level;
use turbo_tasks::{keyed_cell, KeyedCellContext, ReadRef, TryJoinIterExt, ValueToString, Vc};

use super::{Chunk, ChunkItem, ChunkItems, ChunkType, ChunkingContext};
use crate::{
    module::Module,
    output::{OutputAsset, OutputAssets},
};

#[tracing::instrument(level = Level::TRACE, skip_all)]
pub async fn make_chunks(
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    chunk_items: impl IntoIterator<Item = Vc<Box<dyn ChunkItem>>>,
    referenced_output_assets: Vec<Vc<Box<dyn OutputAsset>>>,
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
    let mut map = IndexMap::<_, Vec<_>>::new();
    for (ty, chunk_item) in chunk_items {
        map.entry(ty).or_default().push(chunk_item);
    }

    let mut referenced_output_assets = Vc::cell(referenced_output_assets);
    let other_referenced_output_assets = Vc::cell(Vec::new());

    let cell_context = KeyedCellContext::new();

    let mut chunks = Vec::new();
    for (ty, chunk_items) in map {
        let ty_name = ty.to_string().await?.clone_value();

        let chunk_items = chunk_items
            .into_iter()
            .map(|chunk_item| async move {
                Ok((
                    chunk_item,
                    *ty.chunk_item_size(chunking_context, chunk_item, chunk_group_root)
                        .await?,
                    chunk_item.asset_ident().to_string().await?,
                ))
            })
            .try_join()
            .await?;

        let mut split_context = SplitContext {
            ty,
            cell_context,
            chunking_context,
            chunk_group_root,
            chunks: &mut chunks,
            referenced_output_assets: &mut referenced_output_assets,
            empty_referenced_output_assets: other_referenced_output_assets,
        };

        app_vendors_split(chunk_items, ty_name, &mut split_context).await?;
    }

    Ok(chunks)
}

type ChunkItemWithInfo = (Vc<Box<dyn ChunkItem>>, usize, ReadRef<String>);

struct SplitContext<'a> {
    ty: Vc<Box<dyn ChunkType>>,
    cell_context: Vc<KeyedCellContext>,
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    chunk_group_root: Option<Vc<Box<dyn Module>>>,
    chunks: &'a mut Vec<Vc<Box<dyn Chunk>>>,
    referenced_output_assets: &'a mut Vc<OutputAssets>,
    empty_referenced_output_assets: Vc<OutputAssets>,
}

async fn handle_split_group(
    chunk_items: &mut Vec<ChunkItemWithInfo>,
    key: &mut String,
    split_context: &mut SplitContext<'_>,
    remaining: Option<&mut Vec<ChunkItemWithInfo>>,
) -> Result<bool> {
    Ok(match (chunk_size(chunk_items), remaining) {
        (ChunkSize::Large, _) => false,
        (ChunkSize::Perfect, _) | (ChunkSize::Small, None) => {
            make_chunk(chunk_items, key, split_context).await?;
            true
        }
        (ChunkSize::Small, Some(remaining)) => {
            remaining.extend(take(chunk_items));
            true
        }
    })
}

#[tracing::instrument(level = Level::TRACE, skip(chunk_items, split_context))]
async fn make_chunk(
    chunk_items: &[ChunkItemWithInfo],
    key: &mut String,
    split_context: &mut SplitContext<'_>,
) -> Result<()> {
    split_context.chunks.push(
        split_context.ty.chunk(
            split_context.chunking_context,
            keyed_cell(
                split_context.cell_context,
                take(key),
                ChunkItems(
                    chunk_items
                        .iter()
                        .map(|&(chunk_item, ..)| chunk_item)
                        .collect(),
                ),
            )
            .await?,
            replace(
                split_context.referenced_output_assets,
                split_context.empty_referenced_output_assets,
            ),
            split_context.chunk_group_root,
        ),
    );
    Ok(())
}

#[tracing::instrument(level = Level::TRACE, skip(chunk_items, split_context))]
async fn app_vendors_split(
    chunk_items: Vec<ChunkItemWithInfo>,
    mut name: String,
    split_context: &mut SplitContext<'_>,
) -> Result<()> {
    let mut app_chunk_items = Vec::new();
    let mut vendors_chunk_items = Vec::new();
    for (chunk_item, size, asset_ident) in chunk_items {
        if is_app_code(&*asset_ident) {
            app_chunk_items.push((chunk_item, size, asset_ident));
        } else {
            vendors_chunk_items.push((chunk_item, size, asset_ident));
        }
    }
    let mut remaining = Vec::new();
    let mut key = format!("{}-app", name);
    if !handle_split_group(
        &mut app_chunk_items,
        &mut key,
        split_context,
        Some(&mut remaining),
    )
    .await?
    {
        package_name_split(app_chunk_items, key, split_context).await?;
    }
    let mut key = format!("{}-vendors", name);
    if !handle_split_group(
        &mut vendors_chunk_items,
        &mut key,
        split_context,
        Some(&mut remaining),
    )
    .await?
    {
        package_name_split(vendors_chunk_items, key, split_context).await?;
    }
    if !remaining.is_empty() {
        if !handle_split_group(&mut remaining, &mut name, split_context, None).await? {
            package_name_split(remaining, name, split_context).await?;
        }
    }
    Ok(())
}

#[tracing::instrument(level = Level::TRACE, skip(chunk_items, split_context))]
async fn package_name_split(
    chunk_items: Vec<ChunkItemWithInfo>,
    mut name: String,
    split_context: &mut SplitContext<'_>,
) -> Result<()> {
    let mut map = IndexMap::<_, Vec<ChunkItemWithInfo>>::new();
    for (chunk_item, size, asset_ident) in chunk_items {
        let package_name = package_name(&*asset_ident);
        if let Some(list) = map.get_mut(package_name) {
            list.push((chunk_item, size, asset_ident));
        } else {
            map.insert(
                package_name.to_string(),
                vec![(chunk_item, size, asset_ident)],
            );
        }
    }
    let mut remaining = Vec::new();
    for (package_name, mut list) in map {
        let mut key = format!("{}-{}", name, package_name);
        if !handle_split_group(&mut list, &mut key, split_context, Some(&mut remaining)).await? {
            folder_split(list, 0, key, split_context).await?;
        }
    }
    if !remaining.is_empty() {
        if !handle_split_group(&mut remaining, &mut name, split_context, None).await? {
            folder_split(remaining, 0, name, split_context).await?;
        }
    }
    Ok(())
}

fn folder_split_boxed<'a, 'b>(
    chunk_items: Vec<ChunkItemWithInfo>,
    location: usize,
    name: String,
    split_context: &'a mut SplitContext<'b>,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(folder_split(chunk_items, location, name, split_context))
}

#[tracing::instrument(level = Level::TRACE, skip(chunk_items, split_context))]
async fn folder_split(
    mut chunk_items: Vec<ChunkItemWithInfo>,
    mut location: usize,
    mut name: String,
    split_context: &mut SplitContext<'_>,
) -> Result<()> {
    let mut map = IndexMap::<_, (_, Vec<ChunkItemWithInfo>)>::new();
    loop {
        for (chunk_item, size, asset_ident) in chunk_items {
            let (folder_name, new_location) = folder_name(&*asset_ident, location);
            if let Some((_, list)) = map.get_mut(folder_name) {
                list.push((chunk_item, size, asset_ident));
            } else {
                map.insert(
                    folder_name.to_string(),
                    (new_location, vec![(chunk_item, size, asset_ident)]),
                );
            }
        }
        if map.len() == 1 {
            // shortcut
            let (folder_name, (new_location, list)) = map.into_iter().next().unwrap();
            if let Some(new_location) = new_location {
                chunk_items = list;
                location = new_location;
                map = IndexMap::new();
                continue;
            } else {
                let mut key = format!("{}-{}", name, folder_name);
                make_chunk(&list, &mut key, split_context).await?;
                return Ok(());
            }
        } else {
            break;
        }
    }
    let mut remaining = Vec::new();
    for (folder_name, (new_location, mut list)) in map {
        let mut key = format!("{}-{}", name, folder_name);
        if !handle_split_group(&mut list, &mut key, split_context, Some(&mut remaining)).await? {
            if let Some(new_location) = new_location {
                folder_split_boxed(list, new_location, key, split_context).await?;
            } else {
                make_chunk(&list, &mut key, split_context).await?;
            }
        }
    }
    if !remaining.is_empty() {
        if !handle_split_group(&mut remaining, &mut name, split_context, None).await? {
            make_chunk(&remaining, &mut name, split_context).await?;
        }
    }
    Ok(())
}

fn is_app_code(ident: &str) -> bool {
    !ident.contains("/node_modules/")
}
fn package_name(ident: &str) -> &str {
    static PACKAGE_NAME_REGEX: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"/node_modules/((?:@[^/]+/)?[^/]+)").unwrap());
    if let Some(result) = PACKAGE_NAME_REGEX.find_iter(&ident).last() {
        &result.as_str()["/node_modules/".len()..]
    } else {
        ""
    }
}
fn folder_name(ident: &str, location: usize) -> (&str, Option<usize>) {
    if let Some(offset) = ident[location..].find('/') {
        let new_location = location + offset + 1;
        (&ident[..new_location], Some(new_location))
    } else {
        (ident, None)
    }
}

const LARGE_CHUNK: usize = 100_000;
const SMALL_CHUNK: usize = 10_000;

enum ChunkSize {
    Large,
    Perfect,
    Small,
}

fn chunk_size(chunk_items: &[ChunkItemWithInfo]) -> ChunkSize {
    let mut total_size = 0;
    for (_, size, _) in chunk_items {
        total_size += size;
    }
    if total_size >= LARGE_CHUNK {
        ChunkSize::Large
    } else if total_size > SMALL_CHUNK {
        ChunkSize::Perfect
    } else {
        ChunkSize::Small
    }
}
