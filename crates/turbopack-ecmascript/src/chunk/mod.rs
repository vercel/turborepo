pub(crate) mod chunk_type;
pub(crate) mod content;
pub(crate) mod context;
pub(crate) mod data;
pub(crate) mod esm_scope;
pub(crate) mod item;
pub(crate) mod placeable;

use std::fmt::Write;

use anyhow::{bail, Result};
use indexmap::IndexSet;
use turbo_tasks::{TryJoinIterExt, Value, ValueToString, Vc};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::{Chunk, ChunkItem, ChunkingContext, Chunks, ModuleIds},
    ident::AssetIdent,
    introspect::{
        module::IntrospectableModule,
        utils::{children_from_output_assets, content_to_details},
        Introspectable, IntrospectableChildren,
    },
    module::Module,
    output::OutputAssets,
    reference::ModuleReference,
};

pub use self::{
    chunk_type::EcmascriptChunkType,
    content::EcmascriptChunkContent,
    context::EcmascriptChunkingContext,
    data::EcmascriptChunkData,
    item::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkItemExt,
        EcmascriptChunkItemOptions,
    },
    placeable::{EcmascriptChunkPlaceable, EcmascriptChunkPlaceables, EcmascriptExports},
};

#[turbo_tasks::value]
pub struct EcmascriptChunk {
    pub chunking_context: Vc<Box<dyn EcmascriptChunkingContext>>,
    pub ident: Vc<AssetIdent>,
    pub content: Vc<EcmascriptChunkContent>,
}

#[turbo_tasks::value(transparent)]
pub struct EcmascriptChunks(Vec<Vc<EcmascriptChunk>>);

#[turbo_tasks::value_impl]
impl EcmascriptChunk {
    #[turbo_tasks::function]
    pub async fn new(
        chunking_context: Vc<Box<dyn EcmascriptChunkingContext>>,
        ident: Vc<AssetIdent>,
        content: Vc<EcmascriptChunkContent>,
    ) -> Result<Vc<Self>> {
        Ok(EcmascriptChunk {
            chunking_context,
            ident,
            content,
        }
        .cell())
    }

    #[turbo_tasks::function]
    pub async fn entry_ids(self: Vc<Self>) -> Result<Vc<ModuleIds>> {
        // TODO return something usefull
        Ok(Vc::cell(Default::default()))
    }

    #[turbo_tasks::function]
    pub async fn compare(
        left: Vc<EcmascriptChunk>,
        right: Vc<EcmascriptChunk>,
    ) -> Result<Vc<EcmascriptChunkComparison>> {
        let a = left.await?;
        let b = right.await?;

        let a = a.content;
        let b = b.content;

        let a: IndexSet<_> = a.await?.chunk_items.iter().copied().collect();
        let b: IndexSet<_> = b.await?.chunk_items.iter().copied().collect();

        let mut unshared_a = a.clone();
        let mut unshared_b = b.clone();
        let mut shared = IndexSet::new();
        for item in b {
            if unshared_a.remove(&item) {
                shared.insert(item);
            }
        }
        for item in &shared {
            unshared_b.remove(item);
        }
        Ok(EcmascriptChunkComparison {
            shared_chunk_items: shared.len(),
            left_chunk_items: unshared_a.len(),
            right_chunk_items: unshared_b.len(),
        }
        .cell())
    }
}

#[turbo_tasks::value]
pub struct EcmascriptChunkComparison {
    pub shared_chunk_items: usize,
    pub left_chunk_items: usize,
    pub right_chunk_items: usize,
}

#[turbo_tasks::function]
fn chunk_item_key() -> Vc<String> {
    Vc::cell("chunk item".to_string())
}

#[turbo_tasks::function]
fn availability_root_key() -> Vc<String> {
    Vc::cell("current_availability_root".to_string())
}

#[turbo_tasks::value_impl]
impl Chunk for EcmascriptChunk {
    #[turbo_tasks::function]
    async fn ident(self: Vc<Self>) -> Result<Vc<AssetIdent>> {
        let this = self.await?;

        let mut ident = this.ident.await?.clone_value();

        let EcmascriptChunkContent {
            chunk_items,
            availability_info,
            ..
        } = &*this.content.await?;

        // The included chunk items and the availability info describe the chunk
        // uniquely
        let chunk_item_key = chunk_item_key();
        for &chunk_item in chunk_items.iter() {
            ident
                .assets
                .push((chunk_item_key, chunk_item.asset_ident()));
        }

        // Current availability root is included
        if let Some(current_availability_root) = availability_info.current_availability_root() {
            let root_ident = current_availability_root.ident();
            ident.assets.push((availability_root_key(), root_ident));
        }

        // Available assets are included
        if let Some(available_modules) = availability_info.available_modules() {
            ident
                .modifiers
                .push(Vc::cell(available_modules.hash().await?.to_string()));
        }

        // Make sure the idents are resolved
        for (_, ident) in ident.assets.iter_mut() {
            *ident = ident.resolve().await?;
        }

        Ok(AssetIdent::new(Value::new(ident)))
    }

    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
        Vc::upcast(self.chunking_context)
    }

    #[turbo_tasks::function]
    async fn parallel_chunks(&self) -> Result<Vc<Chunks>> {
        let content = self.content.await?;
        let mut chunks = Vec::new();
        for chunk in content.chunks.iter() {
            chunks.push(*chunk);
        }
        Ok(Vc::cell(chunks))
    }

    #[turbo_tasks::function]
    async fn references(self: Vc<Self>) -> Result<Vc<OutputAssets>> {
        let this = self.await?;
        let content = this.content.await?;
        let mut references = Vec::new();
        let assets = content
            .external_module_references
            .iter()
            .map(|r| r.resolve_reference().primary_output_assets())
            .try_join()
            .await?;
        for &output_asset in assets.iter().flatten() {
            references.push(output_asset);
        }

        Ok(Vc::cell(references))
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptChunk {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<Vc<String>> {
        Ok(Vc::cell(format!("chunk {}", self.ident.to_string().await?)))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunk {
    #[turbo_tasks::function]
    pub fn chunk_content(&self) -> Vc<EcmascriptChunkContent> {
        self.content
    }

    #[turbo_tasks::function]
    pub async fn chunk_items_count(&self) -> Result<Vc<usize>> {
        Ok(Vc::cell(self.content.await?.chunk_items.len()))
    }
}

#[turbo_tasks::value_impl]
impl Asset for EcmascriptChunk {
    #[turbo_tasks::function]
    fn content(self: Vc<Self>) -> Result<Vc<AssetContent>> {
        bail!("EcmascriptChunk::content() is not implemented")
    }
}

#[turbo_tasks::function]
fn introspectable_type() -> Vc<String> {
    Vc::cell("ecmascript chunk".to_string())
}

#[turbo_tasks::function]
fn chunk_item_module_key() -> Vc<String> {
    Vc::cell("module".to_string())
}

#[turbo_tasks::value_impl]
impl Introspectable for EcmascriptChunk {
    #[turbo_tasks::function]
    fn ty(&self) -> Vc<String> {
        introspectable_type()
    }

    #[turbo_tasks::function]
    fn title(self: Vc<Self>) -> Vc<String> {
        self.path().to_string()
    }

    #[turbo_tasks::function]
    async fn details(self: Vc<Self>) -> Result<Vc<String>> {
        let content = content_to_details(self.content());
        let mut details = String::new();
        let this = self.await?;
        let chunk_content = this.content.await?;
        details += "Chunk items:\n\n";
        for chunk_item in chunk_content.chunk_items.iter() {
            writeln!(details, "- {}", chunk_item.asset_ident().to_string().await?)?;
        }
        details += "\nContent:\n\n";
        write!(details, "{}", content.await?)?;
        Ok(Vc::cell(details))
    }

    #[turbo_tasks::function]
    async fn children(self: Vc<Self>) -> Result<Vc<IntrospectableChildren>> {
        let mut children = children_from_output_assets(self.references())
            .await?
            .clone_value();
        let chunk_item_module_key = chunk_item_module_key();
        for &chunk_item in self.await?.content.await?.chunk_items.iter() {
            children.insert((
                chunk_item_module_key,
                IntrospectableModule::new(chunk_item.module()),
            ));
        }
        Ok(Vc::cell(children))
    }
}
