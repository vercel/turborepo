use anyhow::Result;
use turbo_tasks::Vc;
use turbo_tasks_fs::rope::RopeBuilder;
use turbopack_core::{
    chunk::{AsyncModuleInfo, ChunkItem, ChunkType, ChunkingContext},
    ident::AssetIdent,
    module::Module,
    reference::ModuleReferences,
};

use super::module::{EcmascriptModuleReexportsPartModule, EcmascriptModuleReexportsPartModuleType};
use crate::{
    chunk::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkItemOptions,
        EcmascriptChunkPlaceable, EcmascriptChunkType, EcmascriptChunkingContext,
    },
    EcmascriptModuleContent,
};

#[turbo_tasks::value(shared)]
pub struct EcmascriptModuleReexportsPartChunkItem {
    pub(super) module: Vc<EcmascriptModuleReexportsPartModule>,
    pub(super) chunking_context: Vc<Box<dyn EcmascriptChunkingContext>>,
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for EcmascriptModuleReexportsPartChunkItem {
    #[turbo_tasks::function]
    fn content(self: Vc<Self>) -> Vc<EcmascriptChunkItemContent> {
        panic!("content() should never be called");
    }

    #[turbo_tasks::function]
    async fn content_with_async_module_info(
        &self,
        async_module_info: Option<Vc<AsyncModuleInfo>>,
    ) -> Result<Vc<EcmascriptChunkItemContent>> {
        let module = self.module.await?;
        let chunking_context = self.chunking_context;
        let exports = self.module.get_exports();
        let original_module = module.module;
        match module.ty {
            EcmascriptModuleReexportsPartModuleType::Locals => {
                let async_module_options = original_module
                    .get_async_module()
                    .module_options(async_module_info);
                let parsed = original_module.parse().resolve().await?;

                let mut analyze_result = original_module.analyze().await?.clone_value();
                analyze_result.exports = exports;
                analyze_result.reexport_references = Vc::cell(vec![]);
                let analyze_result = analyze_result.cell();

                let content = EcmascriptModuleContent::new(
                    parsed,
                    self.module.ident(),
                    chunking_context,
                    analyze_result,
                    async_module_info,
                );
                Ok(EcmascriptChunkItemContent::new(
                    content,
                    self.chunking_context,
                    async_module_options,
                ))
            }
            EcmascriptModuleReexportsPartModuleType::ReexportsFacade => {
                let mut code = RopeBuilder::default();

                code.push_static_bytes(b"// TODO");

                Ok(EcmascriptChunkItemContent {
                    inner_code: code.build(),
                    source_map: None,
                    options: EcmascriptChunkItemOptions {
                        strict: true,
                        ..Default::default()
                    },
                    ..Default::default()
                }
                .cell())
            }
        }
    }

    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn EcmascriptChunkingContext>> {
        self.chunking_context
    }
}

#[turbo_tasks::value_impl]
impl ChunkItem for EcmascriptModuleReexportsPartChunkItem {
    #[turbo_tasks::function]
    fn references(&self) -> Vc<ModuleReferences> {
        self.module.references()
    }

    #[turbo_tasks::function]
    fn asset_ident(&self) -> Result<Vc<AssetIdent>> {
        Ok(self.module.ident())
    }

    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
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
