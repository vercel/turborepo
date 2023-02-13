use anyhow::{bail, Result};
use indexmap::IndexSet;
use turbo_tasks::TryJoinIterExt;
use turbopack_core::chunk::{
    optimize::{ChunkOptimizer, ChunkOptimizerVc},
    ChunkGroupVc, ChunkVc, ChunkingContextVc, ChunksVc,
};

use super::{CssChunkPlaceablesVc, CssChunkVc};

#[turbo_tasks::value]
pub struct CssChunkOptimizer(ChunkingContextVc);

#[turbo_tasks::value_impl]
impl CssChunkOptimizerVc {
    #[turbo_tasks::function]
    pub fn new(context: ChunkingContextVc) -> Self {
        CssChunkOptimizer(context).cell()
    }
}

#[turbo_tasks::value_impl]
impl ChunkOptimizer for CssChunkOptimizer {
    #[turbo_tasks::function]
    async fn optimize(&self, chunks: ChunksVc, _chunk_group: ChunkGroupVc) -> Result<ChunksVc> {
        // The CSS optimizer works under the constraint that the order in which
        // CSS chunks are loaded must be preserved, as CSS rules
        // precedence is determined by the order in which they are
        // loaded. This means that we may not merge chunks that are not
        // adjacent to each other in a valid reverse topological order.

        // TODO(alexkirsz) It might be more interesting to only merge adjacent
        // chunks when they are part of the same chunk subgraph.
        // However, the optimizer currently does not have access to this
        // information, as chunks are already fully flattened by the
        // time they reach the optimizer.

        merge_adjacent_chunks(chunks).await
    }
}

async fn css(chunk: ChunkVc) -> Result<CssChunkVc> {
    if let Some(chunk) = CssChunkVc::resolve_from(chunk).await? {
        Ok(chunk)
    } else {
        bail!("CssChunkOptimizer can only be used on CssChunks")
    }
}

async fn merge_chunks(
    first: CssChunkVc,
    chunks: impl IntoIterator<Item = &CssChunkVc>,
) -> Result<CssChunkVc> {
    let chunks = chunks.into_iter().copied().try_join().await?;
    let main_entries = chunks
        .iter()
        .map(|c| c.main_entries)
        .try_join()
        .await?
        .iter()
        .flat_map(|e| e.iter().copied())
        .collect::<IndexSet<_>>();
    Ok(CssChunkVc::new_normalized(
        first.await?.context,
        CssChunkPlaceablesVc::cell(main_entries.into_iter().collect()),
    ))
}

/// The maximum number of chunks to merge into a single chunk.
const CHUNK_MERGE_COUNT: usize = 25;

async fn aggregate_adjacent_chunks(
    chunks: ChunksVc,
    include_chunk: impl Fn(ChunkVc, &[ChunkVc]) -> bool,
) -> Result<Vec<Vec<ChunkVc>>> {
    let mut chunks_vecs = vec![];
    let chunks = chunks.await?;
    let mut chunks_iter = chunks.iter();

    let Some(first) = chunks_iter.next() else {
        return Ok(vec![]);
    };

    let mut current_chunks = vec![*first];

    for chunk in chunks_iter {
        if !include_chunk(*chunk, &current_chunks) {
            chunks_vecs.push(std::mem::take(&mut current_chunks))
        }

        current_chunks.push(*chunk);
    }

    chunks_vecs.push(current_chunks);

    Ok(chunks_vecs)
}

async fn merge_adjacent_chunks(chunks: ChunksVc) -> Result<ChunksVc> {
    let chunks = aggregate_adjacent_chunks(chunks, |_chunk, current_chunks| {
        current_chunks.len() < CHUNK_MERGE_COUNT
    })
    .await?;

    let chunks = chunks
        .into_iter()
        .map(|chunks| async move {
            let chunks = chunks.iter().copied().map(css).try_join().await?;
            merge_chunks(*chunks.first().unwrap(), &chunks).await
        })
        .try_join()
        .await?
        .into_iter()
        .map(|chunk| chunk.as_chunk())
        .collect();

    Ok(ChunksVc::cell(chunks))
}
