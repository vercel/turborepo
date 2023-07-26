//! WebAssembly support for turbopack.
//!
//! WASM assets are copied directly to the output folder.
//!
//! When imported from ES modules, they produce a thin module that loads and
//! instantiates the WebAssembly module.

#![feature(min_specialization)]
#![feature(arbitrary_self_types)]
#![feature(async_fn_in_trait)]

mod loader;
pub mod url;

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use indexmap::indexmap;
use turbo_tasks::{Value, Vc};
use turbo_tasks_fs::{File, FileContent, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::{
        availability_info::AvailabilityInfo, Chunk, ChunkItem, ChunkableModule, ChunkingContext,
    },
    context::AssetContext,
    ident::AssetIdent,
    module::{Module, OptionModule},
    output::OutputAsset,
    reference::ModuleReferences,
    reference_type::ReferenceType,
    resolve::{origin::ResolveOrigin, parse::Request},
    source::Source,
    virtual_source::VirtualSource,
};
use turbopack_ecmascript::{
    chunk::{
        EcmascriptChunk, EcmascriptChunkItem, EcmascriptChunkItemContent,
        EcmascriptChunkItemOptions, EcmascriptChunkPlaceable, EcmascriptChunkingContext,
        EcmascriptExports,
    },
    references::async_module::OptionAsyncModule,
    EcmascriptModuleAsset,
};
use wasmparser::{Chunk as WasmChunk, Parser, Payload};

use crate::{loader::loader_source, url::WebAssemblyUrlModuleAsset};

#[turbo_tasks::function]
fn modifier() -> Vc<String> {
    Vc::cell("wasm".to_string())
}

#[turbo_tasks::function]
async fn binary_source(source: Vc<Box<dyn Source>>) -> Result<Vc<Box<dyn Source>>> {
    let ext = source.ident().path().extension().await?;
    if *ext == "wasm" {
        return Ok(source);
    }

    let content = source.content().file_content().await?;

    let file_content: Vc<FileContent> = if let FileContent::Content(file) = &*content {
        let bytes = file.content().to_bytes()?;

        let parsed = wat::parse_bytes(&bytes)?;

        File::from(&*parsed).into()
    } else {
        FileContent::NotFound.cell()
    };

    Ok(Vc::upcast(VirtualSource::new(
        source.ident().path().append("_.wasm".to_string()),
        AssetContent::file(file_content),
    )))
}

#[turbo_tasks::value]
#[derive(Clone)]
pub struct WebAssemblyModuleAsset {
    pub source: Vc<Box<dyn Source>>,
    pub context: Vc<Box<dyn AssetContext>>,
}

#[turbo_tasks::value_impl]
impl WebAssemblyModuleAsset {
    #[turbo_tasks::function]
    pub fn new(source: Vc<Box<dyn Source>>, context: Vc<Box<dyn AssetContext>>) -> Vc<Self> {
        Self::cell(WebAssemblyModuleAsset { source, context })
    }

    #[turbo_tasks::function]
    fn wasm_asset(&self, context: Vc<Box<dyn ChunkingContext>>) -> Vc<WebAssemblyAsset> {
        WebAssemblyAsset {
            context,
            source: binary_source(self.source),
        }
        .cell()
    }

    #[turbo_tasks::function]
    async fn loader(self: Vc<Self>) -> Result<Vc<EcmascriptModuleAsset>> {
        let this = self.await?;

        let module = this.context.process(
            loader_source(binary_source(this.source).ident(), self.analyze()),
            Value::new(ReferenceType::Internal(Vc::cell(indexmap! {
                "WASM_PATH".to_string() => Vc::upcast(WebAssemblyUrlModuleAsset::new(this.source, this.context)),
            }))),
        );

        let Some(esm_asset) =
            Vc::try_resolve_downcast_type::<EcmascriptModuleAsset>(module).await?
        else {
            bail!("WASM loader was not processed into an EcmascriptModuleAsset");
        };

        Ok(esm_asset)
    }

    #[turbo_tasks::function]
    async fn analyze(&self) -> Result<Vc<WebAssemblyAnalysis>> {
        let content = binary_source(self.source).content().file_content().await?;

        let mut analysis = WebAssemblyAnalysis::default();

        let FileContent::Content(file) = &*content else {
            return Ok(analysis.cell());
        };

        let mut bytes = &*file.content().to_bytes()?;

        let mut parser = Parser::new(0);
        loop {
            let payload = match parser.parse(bytes, true)? {
                WasmChunk::Parsed { consumed, payload } => {
                    bytes = &bytes[consumed..];
                    payload
                }
                // this state isn't possible with `eof = true`
                WasmChunk::NeedMoreData(_) => unreachable!(),
            };

            match payload {
                Payload::ImportSection(s) => {
                    for import in s {
                        let import = import?;

                        analysis
                            .imports
                            .entry(import.module.to_string())
                            .or_default()
                            .push(import.name.to_string());
                    }
                }
                Payload::ExportSection(s) => {
                    for export in s {
                        let export = export?;

                        analysis.exports.push(export.name.to_string());
                    }
                }

                // skip over code sections
                Payload::CodeSectionStart { size, .. } => {
                    parser.skip_section();
                    bytes = &bytes[size as usize..];
                }

                Payload::End(_) => break,
                _ => {}
            }
        }

        Ok(analysis.cell())
    }
}

#[turbo_tasks::value]
#[derive(Default)]
struct WebAssemblyAnalysis {
    imports: BTreeMap<String, Vec<String>>,
    exports: Vec<String>,
}

#[turbo_tasks::value_impl]
impl Module for WebAssemblyModuleAsset {
    #[turbo_tasks::function]
    fn ident(&self) -> Vc<AssetIdent> {
        self.source.ident().with_modifier(modifier())
    }

    #[turbo_tasks::function]
    async fn references(self: Vc<Self>) -> Vc<ModuleReferences> {
        self.loader().references()
    }
}

#[turbo_tasks::value_impl]
impl Asset for WebAssemblyModuleAsset {
    #[turbo_tasks::function]
    fn content(&self) -> Vc<AssetContent> {
        self.source.content()
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModule for WebAssemblyModuleAsset {
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
impl EcmascriptChunkPlaceable for WebAssemblyModuleAsset {
    #[turbo_tasks::function]
    fn as_chunk_item(
        self: Vc<Self>,
        context: Vc<Box<dyn EcmascriptChunkingContext>>,
    ) -> Vc<Box<dyn EcmascriptChunkItem>> {
        Vc::upcast(
            ModuleChunkItem {
                module: self,
                context,
            }
            .cell(),
        )
    }

    #[turbo_tasks::function]
    fn get_exports(self: Vc<Self>) -> Vc<EcmascriptExports> {
        self.loader().get_exports()
    }

    #[turbo_tasks::function]
    fn get_async_module(self: Vc<Self>) -> Vc<OptionAsyncModule> {
        self.loader().get_async_module()
    }
}

#[turbo_tasks::value_impl]
impl ResolveOrigin for WebAssemblyModuleAsset {
    #[turbo_tasks::function]
    fn origin_path(&self) -> Vc<FileSystemPath> {
        self.source.ident().path()
    }

    #[turbo_tasks::function]
    fn context(&self) -> Vc<Box<dyn AssetContext>> {
        self.context
    }

    #[turbo_tasks::function]
    fn get_inner_asset(self: Vc<Self>, request: Vc<Request>) -> Vc<OptionModule> {
        self.loader().get_inner_asset(request)
    }
}

#[turbo_tasks::value]
struct WebAssemblyAsset {
    context: Vc<Box<dyn ChunkingContext>>,
    source: Vc<Box<dyn Source>>,
}

#[turbo_tasks::value_impl]
impl OutputAsset for WebAssemblyAsset {
    #[turbo_tasks::function]
    async fn ident(&self) -> Result<Vc<AssetIdent>> {
        let ident = self.source.ident().with_modifier(modifier());

        let asset_path = self.context.chunk_path(ident, ".wasm".to_string());

        Ok(AssetIdent::from_path(asset_path))
    }
}

#[turbo_tasks::value_impl]
impl Asset for WebAssemblyAsset {
    #[turbo_tasks::function]
    fn content(&self) -> Vc<AssetContent> {
        self.source.content()
    }
}

#[turbo_tasks::value]
struct ModuleChunkItem {
    module: Vc<WebAssemblyModuleAsset>,
    context: Vc<Box<dyn EcmascriptChunkingContext>>,
}

#[turbo_tasks::value_impl]
impl ChunkItem for ModuleChunkItem {
    #[turbo_tasks::function]
    fn asset_ident(&self) -> Vc<AssetIdent> {
        self.module.ident()
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<Vc<ModuleReferences>> {
        let loader = self.module.loader().as_chunk_item(self.context);

        Ok(loader.references())
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for ModuleChunkItem {
    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn EcmascriptChunkingContext>> {
        self.context
    }

    #[turbo_tasks::function]
    fn content(self: Vc<Self>) -> Vc<EcmascriptChunkItemContent> {
        self.content_with_availability_info(Value::new(AvailabilityInfo::Untracked))
    }

    #[turbo_tasks::function]
    async fn content_with_availability_info(
        &self,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<Vc<EcmascriptChunkItemContent>> {
        let loader_asset = self.module.loader();

        let chunk_item_content = loader_asset
            .as_chunk_item(self.context)
            .content_with_availability_info(availability_info)
            .await?;

        Ok(EcmascriptChunkItemContent {
            options: EcmascriptChunkItemOptions {
                wasm: true,
                ..chunk_item_content.options.clone()
            },
            ..chunk_item_content.clone_value()
        }
        .into())
    }
}

pub fn register() {
    turbo_tasks::register();
    turbo_tasks_fs::register();
    turbopack_core::register();
    turbopack_ecmascript::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}
