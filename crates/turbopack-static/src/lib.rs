//! Static asset support for turbopack.
//!
//! Static assets are copied directly to the output folder.
//!
//! When imported from ES modules, they produce a thin module that simply
//! exports the asset's path.
//!
//! When referred to from CSS assets, the reference is replaced with the asset's
//! path.

#![feature(min_specialization)]
#![feature(arbitrary_self_types)]

pub mod fixed;
pub mod output_asset;

use anyhow::Result;
use turbo_tasks::{ValueToString, Vc};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::{ChunkItem, ChunkType, ChunkableModule, ChunkingContext},
    context::AssetContext,
    ident::AssetIdent,
    module::Module,
    output::OutputAsset,
    reference::{ModuleReferences, SingleOutputAssetReference},
    source::Source,
};
use turbopack_css::embed::CssEmbed;
use turbopack_ecmascript::{
    chunk::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkPlaceable,
        EcmascriptChunkType, EcmascriptExports,
    },
    utils::StringifyJs,
};

use self::output_asset::StaticAsset;

#[turbo_tasks::function]
fn modifier() -> Vc<String> {
    Vc::cell("static".to_string())
}

#[turbo_tasks::value]
#[derive(Clone)]
pub struct StaticModuleAsset {
    pub source: Vc<Box<dyn Source>>,
    pub asset_context: Vc<Box<dyn AssetContext>>,
}

#[turbo_tasks::value_impl]
impl StaticModuleAsset {
    #[turbo_tasks::function]
    pub fn new(source: Vc<Box<dyn Source>>, asset_context: Vc<Box<dyn AssetContext>>) -> Vc<Self> {
        Self::cell(StaticModuleAsset {
            source,
            asset_context,
        })
    }

    #[turbo_tasks::function]
    async fn static_asset(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<StaticAsset>> {
        Ok(StaticAsset::new(chunking_context, self.await?.source))
    }
}

#[turbo_tasks::value_impl]
impl Module for StaticModuleAsset {
    #[turbo_tasks::function]
    fn ident(&self) -> Vc<AssetIdent> {
        self.source
            .ident()
            .with_modifier(modifier())
            .with_layer(self.asset_context.layer())
    }
}

#[turbo_tasks::value_impl]
impl Asset for StaticModuleAsset {
    #[turbo_tasks::function]
    fn content(&self) -> Vc<AssetContent> {
        self.source.content()
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModule for StaticModuleAsset {
    #[turbo_tasks::function]
    async fn as_chunk_item(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<Box<dyn turbopack_core::chunk::ChunkItem>>> {
        Ok(Vc::upcast(ModuleChunkItem::cell(ModuleChunkItem {
            module: self,
            chunking_context,
            static_asset: self.static_asset(Vc::upcast(chunking_context)),
        })))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for StaticModuleAsset {
    #[turbo_tasks::function]
    fn get_exports(&self) -> Vc<EcmascriptExports> {
        EcmascriptExports::Value.into()
    }
}

#[turbo_tasks::value]
struct ModuleChunkItem {
    module: Vc<StaticModuleAsset>,
    chunking_context: Vc<Box<dyn ChunkingContext>>,
    static_asset: Vc<StaticAsset>,
}

#[turbo_tasks::value_impl]
impl ChunkItem for ModuleChunkItem {
    #[turbo_tasks::function]
    fn asset_ident(&self) -> Vc<AssetIdent> {
        self.module.ident()
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<Vc<ModuleReferences>> {
        Ok(Vc::cell(vec![Vc::upcast(SingleOutputAssetReference::new(
            Vc::upcast(self.static_asset),
            Vc::cell(format!(
                "static(url) {}",
                self.static_asset.ident().to_string().await?
            )),
        ))]))
    }

    #[turbo_tasks::function]
    async fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
        Vc::upcast(self.chunking_context)
    }

    #[turbo_tasks::function]
    async fn ty(&self) -> Result<Vc<Box<dyn ChunkType>>> {
        Ok(Vc::upcast(
            Vc::<EcmascriptChunkType>::default().resolve().await?,
        ))
    }

    #[turbo_tasks::function]
    fn module(&self) -> Vc<Box<dyn Module>> {
        Vc::upcast(self.module)
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for ModuleChunkItem {
    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
        self.chunking_context
    }

    #[turbo_tasks::function]
    async fn content(&self) -> Result<Vc<EcmascriptChunkItemContent>> {
        Ok(EcmascriptChunkItemContent {
            inner_code: format!(
                "__turbopack_export_value__({path});",
                path = StringifyJs(
                    &self
                        .chunking_context
                        .asset_url(self.static_asset.ident())
                        .await?
                )
            )
            .into(),
            ..Default::default()
        }
        .into())
    }
}

#[turbo_tasks::value_impl]
impl CssEmbed for ModuleChunkItem {
    #[turbo_tasks::function]
    fn embedded_asset(&self) -> Vc<Box<dyn OutputAsset>> {
        Vc::upcast(self.static_asset)
    }
}

pub fn register() {
    turbo_tasks::register();
    turbo_tasks_fs::register();
    turbopack_core::register();
    turbopack_ecmascript::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}
