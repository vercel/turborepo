pub mod availability_info;
pub mod available_modules;
pub mod chunking;
pub(crate) mod chunking_context;
pub(crate) mod containment_tree;
pub(crate) mod data;
pub(crate) mod evaluate;
pub mod optimize;
pub(crate) mod passthrough_asset;

use std::{
    collections::HashSet,
    fmt::{Debug, Display},
    future::Future,
    hash::Hash,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info_span, Span};
use turbo_tasks::{
    debug::ValueDebugFormat,
    graph::{AdjacencyMap, GraphTraversal, GraphTraversalResult, Visit, VisitControlFlow},
    trace::TraceRawVcs,
    ReadRef, TryFlatJoinIterExt, TryJoinIterExt, Upcast, Value, ValueToString, Vc,
};
use turbo_tasks_fs::FileSystemPath;
use turbo_tasks_hash::DeterministicHash;

use self::availability_info::AvailabilityInfo;
pub use self::{
    chunking_context::{ChunkingContext, ChunkingContextExt},
    data::{ChunkData, ChunkDataOption, ChunksData},
    evaluate::{EvaluatableAsset, EvaluatableAssetExt, EvaluatableAssets},
    passthrough_asset::PassthroughModule,
};
use crate::{
    asset::Asset,
    ident::AssetIdent,
    module::Module,
    output::OutputAssets,
    reference::{ModuleReference, ModuleReferences},
};

/// A module id, which can be a number or string
#[turbo_tasks::value(shared)]
#[derive(Debug, Clone, Hash, Ord, PartialOrd, DeterministicHash)]
#[serde(untagged)]
pub enum ModuleId {
    Number(u32),
    String(String),
}

impl Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleId::Number(i) => write!(f, "{}", i),
            ModuleId::String(s) => write!(f, "{}", s),
        }
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for ModuleId {
    #[turbo_tasks::function]
    fn to_string(&self) -> Vc<String> {
        Vc::cell(self.to_string())
    }
}

impl ModuleId {
    pub fn parse(id: &str) -> Result<ModuleId> {
        Ok(match id.parse::<u32>() {
            Ok(i) => ModuleId::Number(i),
            Err(_) => ModuleId::String(id.to_string()),
        })
    }
}

/// A list of module ids.
#[turbo_tasks::value(transparent, shared)]
pub struct ModuleIds(Vec<Vc<ModuleId>>);

/// A [Module] that can be converted into a [Chunk].
#[turbo_tasks::value_trait]
pub trait ChunkableModule: Module + Asset {
    fn as_chunk_item(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Vc<Box<dyn ChunkItem>>;
}

#[turbo_tasks::value(transparent)]
pub struct Chunks(Vec<Vc<Box<dyn Chunk>>>);

#[turbo_tasks::value_impl]
impl Chunks {
    /// Creates a new empty [Vc<Chunks>].
    #[turbo_tasks::function]
    pub fn empty() -> Vc<Self> {
        Vc::cell(vec![])
    }
}

/// A chunk is one type of asset.
/// It usually contains multiple chunk items.
#[turbo_tasks::value_trait]
pub trait Chunk: Asset {
    fn ident(self: Vc<Self>) -> Vc<AssetIdent>;
    fn chunking_context(self: Vc<Self>) -> Vc<Box<dyn ChunkingContext>>;
    // TODO Once output assets have their own trait, this path() method will move
    // into that trait and ident() will be removed from that. Assets on the
    // output-level only have a path and no complex ident.
    /// The path of the chunk.
    fn path(self: Vc<Self>) -> Vc<FileSystemPath> {
        self.ident().path()
    }

    /// Other [OutputAsset]s referenced from this [Chunk].
    fn references(self: Vc<Self>) -> Vc<OutputAssets> {
        OutputAssets::empty()
    }
}

/// Aggregated information about a chunk content that can be used by the runtime
/// code to optimize chunk loading.
#[turbo_tasks::value(shared)]
#[derive(Default)]
pub struct OutputChunkRuntimeInfo {
    pub included_ids: Option<Vc<ModuleIds>>,
    pub excluded_ids: Option<Vc<ModuleIds>>,
    /// List of paths of chunks containing individual modules that are part of
    /// this chunk. This is useful for selectively loading modules from a chunk
    /// without loading the whole chunk.
    pub module_chunks: Option<Vc<OutputAssets>>,
    pub placeholder_for_future_extensions: (),
}

#[turbo_tasks::value_trait]
pub trait OutputChunk: Asset {
    fn runtime_info(self: Vc<Self>) -> Vc<OutputChunkRuntimeInfo>;
}

/// Specifies how a chunk interacts with other chunks when building a chunk
/// group
#[derive(
    Copy, Default, Clone, Hash, TraceRawVcs, Serialize, Deserialize, Eq, PartialEq, ValueDebugFormat,
)]
pub enum ChunkingType {
    /// Asset is placed in the same chunk group and is loaded in parallel.
    #[default]
    Parallel,
    /// Asset is placed in the same chunk group and is loaded in parallel.
    /// Referenced asset will not inherit the available modules, but form a
    /// new availability root.
    IsolatedParallel,
    /// An async loader is placed into the referencing chunk and loads the
    /// separate chunk group in which the asset is placed.
    Async,
}

#[turbo_tasks::value(transparent)]
pub struct ChunkingTypeOption(Option<ChunkingType>);

/// A [ModuleReference] implementing this trait and returning true for
/// [ChunkableModuleReference::is_chunkable] are considered as potentially
/// chunkable references. When all [Module]s of such a reference implement
/// [ChunkableModule] they are placed in [Chunk]s during chunking.
/// They are even potentially placed in the same [Chunk] when a chunk type
/// specific interface is implemented.
#[turbo_tasks::value_trait]
pub trait ChunkableModuleReference: ModuleReference + ValueToString {
    fn chunking_type(self: Vc<Self>) -> Vc<ChunkingTypeOption> {
        Vc::cell(Some(ChunkingType::default()))
    }
}

pub struct ChunkContentResult {
    pub modules: Vec<Vc<Box<dyn ChunkableModule>>>,
    pub chunk_items: Vec<Vc<Box<dyn ChunkItem>>>,
    pub async_modules: Vec<Vc<Box<dyn ChunkableModule>>>,
    pub external_module_references: Vec<Vc<Box<dyn ModuleReference>>>,
}

pub async fn chunk_content(
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    entries: impl IntoIterator<Item = Vc<Box<dyn Module>>>,
    availability_info: Value<AvailabilityInfo>,
) -> Result<ChunkContentResult> {
    chunk_content_internal_parallel(chunking_context, entries, availability_info).await
}

#[derive(Eq, PartialEq, Clone, Hash)]
enum ChunkContentGraphNode {
    // An asset not placed in the current chunk, but whose references we will
    // follow to find more graph nodes.
    PassthroughModule {
        asset: Vc<Box<dyn Module>>,
    },
    // Chunk items that are placed into the current chunk group
    ChunkItem {
        item: Vc<Box<dyn ChunkItem>>,
        module: Vc<Box<dyn ChunkableModule>>,
        ident: ReadRef<String>,
    },
    // Async module that is referenced from the chunk group
    AsyncModule {
        module: Vc<Box<dyn ChunkableModule>>,
    },
    // Asset that is already available and doesn't need to be included
    AvailableAsset(Vc<Box<dyn Module>>),
    // ModuleReferences that are not placed in the current chunk group
    ExternalModuleReference(Vc<Box<dyn ModuleReference>>),
}

#[derive(Clone, Copy)]
struct ChunkContentContext {
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    availability_info: Value<AvailabilityInfo>,
}

async fn reference_to_graph_nodes(
    chunk_content_context: ChunkContentContext,
    reference: Vc<Box<dyn ModuleReference>>,
) -> Result<
    Vec<(
        Option<(Vc<Box<dyn Module>>, ChunkingType)>,
        ChunkContentGraphNode,
    )>,
> {
    let Some(chunkable_module_reference) =
        Vc::try_resolve_downcast::<Box<dyn ChunkableModuleReference>>(reference).await?
    else {
        return Ok(vec![(
            None,
            ChunkContentGraphNode::ExternalModuleReference(reference),
        )]);
    };

    let Some(chunking_type) = *chunkable_module_reference.chunking_type().await? else {
        return Ok(vec![(
            None,
            ChunkContentGraphNode::ExternalModuleReference(reference),
        )]);
    };

    let modules = reference.resolve_reference().primary_modules().await?;

    let mut graph_nodes = vec![];

    for &module in &modules {
        let module = module.resolve().await?;

        let chunkable_module =
            match Vc::try_resolve_sidecast::<Box<dyn ChunkableModule>>(module).await? {
                Some(chunkable_module) => chunkable_module,
                _ => {
                    return Ok(vec![(
                        None,
                        ChunkContentGraphNode::ExternalModuleReference(reference),
                    )]);
                }
            };

        if let Some(available_modules) = chunk_content_context.availability_info.available_modules()
        {
            if *available_modules.includes(chunkable_module).await? {
                graph_nodes.push((
                    Some((module, chunking_type)),
                    ChunkContentGraphNode::AvailableAsset(module),
                ));
                continue;
            }
        }

        if Vc::try_resolve_sidecast::<Box<dyn PassthroughModule>>(module)
            .await?
            .is_some()
        {
            graph_nodes.push((
                None,
                ChunkContentGraphNode::PassthroughModule { asset: module },
            ));
            continue;
        }

        match chunking_type {
            ChunkingType::Parallel => {
                let chunk_item =
                    chunkable_module.as_chunk_item(chunk_content_context.chunking_context);
                graph_nodes.push((
                    Some((module, chunking_type)),
                    ChunkContentGraphNode::ChunkItem {
                        item: chunk_item,
                        module: chunkable_module,
                        ident: module.ident().to_string().await?,
                    },
                ));
            }
            ChunkingType::IsolatedParallel => {
                todo!();
            }
            ChunkingType::Async => {
                graph_nodes.push((
                    Some((module, chunking_type)),
                    ChunkContentGraphNode::AsyncModule {
                        module: chunkable_module,
                    },
                ));
            }
        }
    }

    Ok(graph_nodes)
}

struct ChunkContentVisit {
    chunk_content_context: ChunkContentContext,
    chunk_items_count: usize,
    processed_assets: HashSet<(ChunkingType, Vc<Box<dyn Module>>)>,
}

type ChunkItemToGraphNodesEdges = impl Iterator<
    Item = (
        Option<(Vc<Box<dyn Module>>, ChunkingType)>,
        ChunkContentGraphNode,
    ),
>;

type ChunkItemToGraphNodesFuture = impl Future<Output = Result<ChunkItemToGraphNodesEdges>>;

impl Visit<ChunkContentGraphNode, ()> for ChunkContentVisit {
    type Edge = (
        Option<(Vc<Box<dyn Module>>, ChunkingType)>,
        ChunkContentGraphNode,
    );
    type EdgesIntoIter = ChunkItemToGraphNodesEdges;
    type EdgesFuture = ChunkItemToGraphNodesFuture;

    fn visit(
        &mut self,
        (option_key, node): (
            Option<(Vc<Box<dyn Module>>, ChunkingType)>,
            ChunkContentGraphNode,
        ),
    ) -> VisitControlFlow<ChunkContentGraphNode, ()> {
        let Some((asset, chunking_type)) = option_key else {
            return VisitControlFlow::Continue(node);
        };

        if !self.processed_assets.insert((chunking_type, asset)) {
            return VisitControlFlow::Skip(node);
        }

        if let ChunkContentGraphNode::ChunkItem { .. } = &node {
            self.chunk_items_count += 1;
        }

        VisitControlFlow::Continue(node)
    }

    fn edges(&mut self, node: &ChunkContentGraphNode) -> Self::EdgesFuture {
        let node = node.clone();

        let chunk_content_context = self.chunk_content_context;

        async move {
            let references = match node {
                ChunkContentGraphNode::PassthroughModule { asset } => asset.references(),
                ChunkContentGraphNode::ChunkItem { item, .. } => item.references(),
                _ => {
                    return Ok(vec![].into_iter().flatten());
                }
            };

            Ok(references
                .await?
                .into_iter()
                .map(|reference| reference_to_graph_nodes(chunk_content_context, *reference))
                .try_join()
                .await?
                .into_iter()
                .flatten())
        }
    }

    fn span(&mut self, node: &ChunkContentGraphNode) -> Span {
        if let ChunkContentGraphNode::ChunkItem { ident, .. } = node {
            info_span!("module", name = display(ident))
        } else {
            Span::current()
        }
    }
}

async fn chunk_content_internal_parallel(
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    entries: impl IntoIterator<Item = Vc<Box<dyn Module>>>,
    availability_info: Value<AvailabilityInfo>,
) -> Result<ChunkContentResult> {
    let root_edges = entries
        .into_iter()
        .map(|entry| async move {
            let entry = entry.resolve().await?;
            let Some(chunkable_module) =
                Vc::try_resolve_downcast::<Box<dyn ChunkableModule>>(entry).await?
            else {
                return Ok(None);
            };
            Ok(Some((
                Some((entry, ChunkingType::Parallel)),
                ChunkContentGraphNode::ChunkItem {
                    item: chunkable_module
                        .as_chunk_item(chunking_context)
                        .resolve()
                        .await?,
                    module: chunkable_module,
                    ident: chunkable_module.ident().to_string().await?,
                },
            )))
        })
        .try_flat_join()
        .await?;

    let chunk_content_context = ChunkContentContext {
        chunking_context,
        availability_info,
    };

    let visit = ChunkContentVisit {
        chunk_content_context,
        chunk_items_count: 0,
        processed_assets: Default::default(),
    };

    let GraphTraversalResult::Completed(traversal_result) =
        AdjacencyMap::new().visit(root_edges, visit).await
    else {
        unreachable!();
    };

    let graph_nodes: Vec<_> = traversal_result?.into_reverse_topological().collect();

    let mut modules = Vec::new();
    let mut chunk_items = Vec::new();
    let mut async_modules = Vec::new();
    let mut external_module_references = Vec::new();

    for graph_node in graph_nodes {
        match graph_node {
            ChunkContentGraphNode::AvailableAsset(_)
            | ChunkContentGraphNode::PassthroughModule { .. } => {}
            ChunkContentGraphNode::ChunkItem { item, module, .. } => {
                chunk_items.push(item);
                modules.push(module);
            }
            ChunkContentGraphNode::AsyncModule { module } => {
                async_modules.push(module);
            }
            ChunkContentGraphNode::ExternalModuleReference(reference) => {
                external_module_references.push(reference);
            }
        }
    }

    Ok(ChunkContentResult {
        modules,
        chunk_items,
        async_modules,
        external_module_references,
    })
}

#[turbo_tasks::value_trait]
pub trait ChunkItem {
    /// The [AssetIdent] of the [Module] that this [ChunkItem] was created from.
    /// For most chunk types this must uniquely identify the asset as it's the
    /// source of the module id used at runtime.
    fn asset_ident(self: Vc<Self>) -> Vc<AssetIdent>;
    /// A [ChunkItem] can describe different `references` than its original
    /// [Module].
    /// TODO(alexkirsz) This should have a default impl that returns empty
    /// references.
    fn references(self: Vc<Self>) -> Vc<ModuleReferences>;

    /// The type of chunk this item should be assembled into.
    fn ty(self: Vc<Self>) -> Vc<Box<dyn ChunkType>>;

    /// A temporary method to retrieve the module associated with this
    /// ChunkItem. TODO: Remove this as part of the chunk refactoring.
    fn module(self: Vc<Self>) -> Vc<Box<dyn Module>>;

    fn chunking_context(self: Vc<Self>) -> Vc<Box<dyn ChunkingContext>>;
}

#[turbo_tasks::value_trait]
pub trait ChunkType {
    /// Create a new chunk for the given chunk items
    fn chunk(
        &self,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
        ident: Vc<AssetIdent>,
        chunk_items: Vc<ChunkItems>,
        referenced_output_assets: Vc<OutputAssets>,
        // TODO This need to go away, it's only needed for EsmScope
        chunk_group_root: Option<Vc<Box<dyn Module>>>,
    ) -> Vc<Box<dyn Chunk>>;
}

#[turbo_tasks::value(transparent)]
pub struct ChunkItems(Vec<Vc<Box<dyn ChunkItem>>>);

pub trait ChunkItemExt: Send {
    /// Returns the module id of this chunk item.
    fn id(self: Vc<Self>) -> Vc<ModuleId>;
}

impl<T> ChunkItemExt for T
where
    T: Upcast<Box<dyn ChunkItem>>,
{
    /// Returns the module id of this chunk item.
    fn id(self: Vc<Self>) -> Vc<ModuleId> {
        let chunk_item = Vc::upcast(self);
        chunk_item.chunking_context().chunk_item_id(chunk_item)
    }
}
