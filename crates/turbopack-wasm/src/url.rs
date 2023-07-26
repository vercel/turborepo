use anyhow::{bail, Result};
use turbo_tasks::{Value, ValueToString, Vc};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::{
        availability_info::AvailabilityInfo, Chunk, ChunkItem, ChunkableModule, ChunkingContext,
    },
    context::AssetContext,
    ident::AssetIdent,
    module::Module,
    output::OutputAsset,
    reference::{ModuleReferences, SingleOutputAssetReference},
    source::Source,
};
use turbopack_ecmascript::{
    chunk::{
        EcmascriptChunk, EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkPlaceable,
        EcmascriptChunkingContext, EcmascriptExports,
    },
    utils::StringifyJs,
};

use crate::{binary_source, WebAssemblyAsset};

#[turbo_tasks::function]
fn modifier() -> Vc<String> {
    Vc::cell("wasm url".to_string())
}

#[turbo_tasks::value]
#[derive(Clone)]
pub struct WebAssemblyUrlModuleAsset {
    pub source: Vc<Box<dyn Source>>,
    pub context: Vc<Box<dyn AssetContext>>,
}

#[turbo_tasks::value_impl]
impl WebAssemblyUrlModuleAsset {
    #[turbo_tasks::function]
    pub fn new(source: Vc<Box<dyn Source>>, context: Vc<Box<dyn AssetContext>>) -> Vc<Self> {
        Self::cell(WebAssemblyUrlModuleAsset { source, context })
    }

    #[turbo_tasks::function]
    fn wasm_asset(&self, context: Vc<Box<dyn ChunkingContext>>) -> Vc<WebAssemblyAsset> {
        WebAssemblyAsset {
            context,
            source: binary_source(self.source),
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl Module for WebAssemblyUrlModuleAsset {
    #[turbo_tasks::function]
    fn ident(&self) -> Vc<AssetIdent> {
        self.source.ident().with_modifier(modifier())
    }
}

#[turbo_tasks::value_impl]
impl Asset for WebAssemblyUrlModuleAsset {
    #[turbo_tasks::function]
    fn content(&self) -> Vc<AssetContent> {
        self.source.content()
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModule for WebAssemblyUrlModuleAsset {
    #[turbo_tasks::function]
    fn as_chunk(
        self: Vc<Self>,
        context: Vc<Box<dyn ChunkingContext>>,
        availability_info: Value<AvailabilityInfo>,
    ) -> Vc<Box<dyn Chunk>> {
        Vc::upcast(EcmascriptChunk::new(
            context,
            Vc::upcast(self),
            availability_info,
        ))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for WebAssemblyUrlModuleAsset {
    #[turbo_tasks::function]
    fn as_chunk_item(
        self: Vc<Self>,
        context: Vc<Box<dyn EcmascriptChunkingContext>>,
    ) -> Vc<Box<dyn EcmascriptChunkItem>> {
        Vc::upcast(
            UrlModuleChunkItem {
                module: self,
                context,
                wasm_asset: self.wasm_asset(Vc::upcast(context)),
            }
            .cell(),
        )
    }

    #[turbo_tasks::function]
    fn get_exports(self: Vc<Self>) -> Vc<EcmascriptExports> {
        EcmascriptExports::Value.cell()
    }
}

#[turbo_tasks::value]
struct UrlModuleChunkItem {
    module: Vc<WebAssemblyUrlModuleAsset>,
    context: Vc<Box<dyn EcmascriptChunkingContext>>,
    wasm_asset: Vc<WebAssemblyAsset>,
}

#[turbo_tasks::value_impl]
impl ChunkItem for UrlModuleChunkItem {
    #[turbo_tasks::function]
    fn asset_ident(&self) -> Vc<AssetIdent> {
        self.module.ident()
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<Vc<ModuleReferences>> {
        Ok(Vc::cell(vec![Vc::upcast(SingleOutputAssetReference::new(
            Vc::upcast(self.wasm_asset),
            Vc::cell(format!(
                "wasm(url) {}",
                self.wasm_asset.ident().to_string().await?
            )),
        ))]))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for UrlModuleChunkItem {
    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn EcmascriptChunkingContext>> {
        self.context
    }

    #[turbo_tasks::function]
    async fn content(&self) -> Result<Vc<EcmascriptChunkItemContent>> {
        let path = self.wasm_asset.ident().path().await?;
        let output_root = self.context.output_root().await?;

        let Some(path) = output_root.get_path_to(&path) else {
            bail!("WASM asset ident is not relative to output root");
        };

        Ok(EcmascriptChunkItemContent {
            inner_code: format!(
                "__turbopack_export_value__({path});",
                path = StringifyJs(path)
            )
            .into(),
            ..Default::default()
        }
        .into())
    }
}
