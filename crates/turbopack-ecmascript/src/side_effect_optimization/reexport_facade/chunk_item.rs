use std::sync::Arc;

use anyhow::{bail, Context, Result};
use swc_core::{
    common::{util::take::Take, Globals, DUMMY_SP, GLOBALS},
    ecma::{
        ast::{Expr, Ident, ModuleItem, Program},
        codegen::{text_writer::JsWriter, Emitter},
        visit::{VisitMutWith, VisitMutWithPath},
    },
    quote,
};
use turbo_tasks::{TryJoinIterExt, Vc};
use turbo_tasks_fs::rope::RopeBuilder;
use turbopack_core::{
    chunk::{
        AsyncModuleInfo, ChunkItem, ChunkItemExt, ChunkType, ChunkableModule, ChunkingContext,
        ModuleId,
    },
    ident::AssetIdent,
    module::Module,
    reference::{ModuleReference, ModuleReferences},
};

use super::module::EcmascriptModuleReexportsFacadeModule;
use crate::{
    chunk::{
        EcmascriptChunkItem, EcmascriptChunkItemContent, EcmascriptChunkItemOptions,
        EcmascriptChunkPlaceable, EcmascriptChunkType, EcmascriptChunkingContext,
        EcmascriptExports,
    },
    code_gen::{CodeGenerateable, CodeGenerateableWithAsyncModuleInfo},
    path_visitor::ApplyVisitors,
    references::esm::base::ReferencedAsset,
    side_effect_optimization::locals::reference::EcmascriptModuleLocalsReference,
};

/// The chunk item for [EcmascriptModuleReexportsFacadeModule].
#[turbo_tasks::value(shared)]
pub struct EcmascriptModuleReexportsFacadeChunkItem {
    pub(super) module: Vc<EcmascriptModuleReexportsFacadeModule>,
    pub(super) chunking_context: Vc<Box<dyn EcmascriptChunkingContext>>,
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for EcmascriptModuleReexportsFacadeChunkItem {
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
        let EcmascriptExports::EsmExports(exports) = *exports.await? else {
            bail!("Expected EsmExports");
        };

        let mut code = RopeBuilder::default();

        let analyze_result = original_module.analyze().await?;

        let mut code_gens = Vec::new();
        for r in analyze_result.reexport_references.await?.iter() {
            let r = r.resolve().await?;
            if let Some(code_gen) =
                Vc::try_resolve_sidecast::<Box<dyn CodeGenerateableWithAsyncModuleInfo>>(r).await?
            {
                code_gens.push(code_gen.code_generation(chunking_context, async_module_info));
            } else if let Some(code_gen) =
                Vc::try_resolve_sidecast::<Box<dyn CodeGenerateable>>(r).await?
            {
                code_gens.push(code_gen.code_generation(chunking_context));
            }
        }

        code_gens.push(exports.code_generation(chunking_context));
        let code_gens = code_gens.into_iter().try_join().await?;
        let code_gens = code_gens.iter().map(|cg| &**cg).collect::<Vec<_>>();

        let mut program = Program::Module(swc_core::ecma::ast::Module::dummy());

        let mut visitors = Vec::new();
        let mut root_visitors = Vec::new();
        for code_gen in code_gens {
            for (path, visitor) in code_gen.visitors.iter() {
                if path.is_empty() {
                    root_visitors.push(&**visitor);
                } else {
                    visitors.push((path, &**visitor));
                }
            }
        }
        let referenced_asset = ReferencedAsset::from_resolve_result(
            EcmascriptModuleLocalsReference::new(module.module).resolve_reference(),
        );
        let referenced_asset = referenced_asset.await?;
        let ident = referenced_asset
            .get_ident()
            .await?
            .context("locals module reference should have an ident")?;

        let ReferencedAsset::Some(module) = *referenced_asset else {
            bail!("locals module reference should have an module reference");
        };
        let id = module
            .as_chunk_item(Vc::upcast(chunking_context))
            .id()
            .await?;

        GLOBALS.set(&Globals::new(), || {
            if !visitors.is_empty() {
                program.visit_mut_with_path(
                    &mut ApplyVisitors::new(visitors),
                    &mut Default::default(),
                );
            }
            for visitor in root_visitors {
                program.visit_mut_with(&mut visitor.create());
            }

            let stmt = quote!(
                "var $name = __turbopack_import__($id);" as Stmt,
                name = Ident::new(ident.into(), DUMMY_SP),
                id: Expr = Expr::Lit(match &*id {
                    ModuleId::String(s) => s.clone().into(),
                    ModuleId::Number(n) => (*n as f64).into(),
                })
            );
            program
                .as_mut_module()
                .unwrap()
                .body
                .push(ModuleItem::Stmt(stmt));
            program.visit_mut_with(&mut swc_core::ecma::transforms::base::hygiene::hygiene());
            program.visit_mut_with(&mut swc_core::ecma::transforms::base::fixer::fixer(None));
        });

        let mut bytes: Vec<u8> = vec![];

        let source_map: Arc<swc_core::common::SourceMap> = Default::default();

        let mut emitter = Emitter {
            cfg: swc_core::ecma::codegen::Config::default(),
            cm: source_map.clone(),
            comments: None,
            wr: JsWriter::new(source_map.clone(), "\n", &mut bytes, None),
        };

        emitter.emit_program(&program)?;

        code.push_bytes(&bytes);

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

    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn EcmascriptChunkingContext>> {
        self.chunking_context
    }
}

#[turbo_tasks::value_impl]
impl ChunkItem for EcmascriptModuleReexportsFacadeChunkItem {
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
