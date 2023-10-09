use std::{collections::HashMap, iter::once};

use anyhow::{Context, Result};
use indexmap::IndexSet;
use petgraph::{algo::tarjan_scc, prelude::DiGraphMap};
use turbo_tasks::{
    graph::{AdjacencyMap, GraphTraversal},
    TryFlatJoinIterExt, TryJoinIterExt, Vc,
};
use turbopack_core::{
    chunk::{ChunkableModuleReference, ChunkingType},
    module::{Module, ModulesSet},
    reference::ModuleReference,
};

use crate::{
    chunk::EcmascriptChunkPlaceable,
    references::esm::{base::ReferencedAsset, EsmAssetReference},
    EcmascriptModuleAssets,
};

/// A graph representing all ESM imports in a chunk group.
#[turbo_tasks::value(serialization = "none", cell = "new", eq = "manual")]
pub(crate) struct EsmScope {
    scc_map: HashMap<Vc<Box<dyn EcmascriptChunkPlaceable>>, Vc<EsmScopeScc>>,
    #[turbo_tasks(trace_ignore, debug_ignore)]
    scc_graph: DiGraphMap<Vc<EsmScopeScc>, ()>,
}

/// Represents a strongly connected component in the EsmScope graph.
///
/// See https://en.wikipedia.org/wiki/Strongly_connected_component
#[turbo_tasks::value(transparent)]
pub(crate) struct EsmScopeScc(Vec<Vc<Box<dyn EcmascriptChunkPlaceable>>>);

#[turbo_tasks::value(transparent)]
pub(crate) struct OptionEsmScopeScc(Option<Vc<EsmScopeScc>>);

#[turbo_tasks::value(transparent)]
pub(crate) struct EsmScopeSccs(Vec<Vc<EsmScopeScc>>);

#[turbo_tasks::value_impl]
impl EsmScope {
    /// Create a new [EsmScope] from the availability root given.
    #[turbo_tasks::function]
    pub(crate) async fn new(chunk_group_root: Vc<Box<dyn Module>>) -> Result<Vc<Self>> {
        let assets = chunkable_modules_set(chunk_group_root);

        let esm_assets = get_ecmascript_module_assets(assets);
        let import_references = collect_import_references(esm_assets).await?;

        let mut graph = DiGraphMap::new();

        for (parent, child) in &*import_references {
            graph.add_edge(*parent, *child, ());
        }

        let sccs = tarjan_scc(&graph);

        let mut scc_map = HashMap::new();
        for scc in sccs {
            let scc_vc = EsmScopeScc(scc.clone()).cell();

            for placeable in scc {
                scc_map.insert(placeable, scc_vc);
            }
        }

        let mut scc_graph = DiGraphMap::new();
        for (parent, child, _) in graph.all_edges() {
            let parent_scc_vc = *scc_map
                .get(&parent)
                .context("unexpected missing SCC in map")?;
            let child_scc_vc = *scc_map
                .get(&child)
                .context("unexpected missing SCC in map")?;

            if parent_scc_vc != child_scc_vc {
                scc_graph.add_edge(parent_scc_vc, child_scc_vc, ());
            }
        }

        Ok(Self::cell(EsmScope { scc_map, scc_graph }))
    }

    /// Gets the [EsmScopeScc] for a given [EcmascriptChunkPlaceable] if it's
    /// part of this graph.
    #[turbo_tasks::function]
    pub(crate) async fn get_scc(
        self: Vc<Self>,
        placeable: Vc<Box<dyn EcmascriptChunkPlaceable>>,
    ) -> Result<Vc<OptionEsmScopeScc>> {
        let this = self.await?;

        Ok(Vc::cell(this.scc_map.get(&placeable).copied()))
    }

    /// Returns all direct children of an [EsmScopeScc].
    #[turbo_tasks::function]
    pub(crate) async fn get_scc_children(
        self: Vc<Self>,
        scc: Vc<EsmScopeScc>,
    ) -> Result<Vc<EsmScopeSccs>> {
        let this = self.await?;

        let children = this.scc_graph.neighbors(scc).collect();

        Ok(Vc::cell(children))
    }
}

#[turbo_tasks::function]
async fn get_ecmascript_module_assets(
    modules: Vc<ModulesSet>,
) -> Result<Vc<EcmascriptModuleAssets>> {
    let esm_assets = modules
        .await?
        .iter()
        .copied()
        .map(|r| async move { anyhow::Ok(Vc::try_resolve_downcast_type(r).await?) })
        .try_flat_join()
        .await?;

    Ok(Vc::cell(esm_assets))
}

// for clippy
type PlaceableVc = Vc<Box<dyn EcmascriptChunkPlaceable>>;

/// A directional reference between 2 [EcmascriptChunkPlaceable]s.
#[turbo_tasks::value(transparent)]
struct ImportReferences(Vec<(PlaceableVc, PlaceableVc)>);

#[turbo_tasks::function]
async fn collect_import_references(
    esm_assets: Vc<EcmascriptModuleAssets>,
) -> Result<Vc<ImportReferences>> {
    let import_references = esm_assets
        .await?
        .iter()
        .copied()
        .map(|a| async move {
            let placeable = Vc::upcast::<Box<dyn EcmascriptChunkPlaceable>>(a)
                .resolve()
                .await?;

            a.references()
                .await?
                .iter()
                .copied()
                .map(|r| async move {
                    let Some(r) = Vc::try_resolve_downcast_type::<EsmAssetReference>(r).await?
                    else {
                        return Ok(None);
                    };

                    let ReferencedAsset::Some(child_placeable) = &*r.get_referenced_asset().await?
                    else {
                        return Ok(None);
                    };

                    let child_placeable = child_placeable.resolve().await?;

                    anyhow::Ok(Some((placeable, child_placeable)))
                })
                .try_flat_join()
                .await
        })
        .try_flat_join()
        .await?;

    Ok(Vc::cell(import_references))
}

// TODO this should be removed
#[turbo_tasks::function]
pub async fn chunkable_modules_set(root: Vc<Box<dyn Module>>) -> Result<Vc<ModulesSet>> {
    let modules = AdjacencyMap::new()
        .skip_duplicates()
        .visit(once(root), |&module: &Vc<Box<dyn Module>>| async move {
            Ok(module
                .references()
                .await?
                .iter()
                .copied()
                .map(|reference| async move {
                    if let Some(chunkable) =
                        Vc::try_resolve_downcast::<Box<dyn ChunkableModuleReference>>(reference)
                            .await?
                    {
                        if matches!(
                            &*chunkable.chunking_type().await?,
                            Some(ChunkingType::Parallel)
                        ) {
                            return Ok(chunkable
                                .resolve_reference()
                                .primary_modules()
                                .await?
                                .clone_value());
                        }
                    }
                    Ok(Vec::new())
                })
                .try_join()
                .await?
                .into_iter()
                .flatten()
                .collect::<IndexSet<_>>())
        })
        .await
        .completed()?;
    Ok(Vc::cell(
        modules.into_inner().into_reverse_topological().collect(),
    ))
}
