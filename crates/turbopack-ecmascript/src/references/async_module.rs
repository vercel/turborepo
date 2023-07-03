use anyhow::Result;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use swc_core::{
    common::DUMMY_SP,
    ecma::ast::{ArrayLit, ArrayPat, Expr, Ident, Pat, Program},
    quote,
};
use turbo_tasks::{primitives::BoolVc, trace::TraceRawVcs, TryJoinIterExt};

use crate::{
    chunk::EcmascriptChunkingContextVc,
    code_gen::{CodeGenerateable, CodeGeneration, CodeGenerationVc},
    create_visitor,
    references::esm::{base::insert_hoisted_stmt, EsmAssetReferenceVc},
    CodeGenerateableVc,
};

#[derive(PartialEq, Eq, Default, Debug, Clone, Serialize, Deserialize, TraceRawVcs)]
pub struct AsyncModuleOptions {
    pub has_top_level_await: bool,
}

#[turbo_tasks::value(transparent)]
pub struct OptionAsyncModuleOptions(Option<AsyncModuleOptions>);

#[turbo_tasks::value_impl]
impl OptionAsyncModuleOptionsVc {
    #[turbo_tasks::function]
    pub(super) async fn is_async(self) -> Result<BoolVc> {
        Ok(BoolVc::cell(self.await?.is_some()))
    }
}

#[turbo_tasks::value(shared)]
pub struct AsyncModule {
    pub(super) references: IndexSet<EsmAssetReferenceVc>,
    pub(super) has_top_level_await: bool,
}

#[turbo_tasks::value(transparent)]
pub struct OptionAsyncModule(Option<AsyncModuleVc>);

#[turbo_tasks::value_impl]
impl AsyncModuleVc {
    #[turbo_tasks::function]
    pub(super) async fn is_async(self) -> Result<BoolVc> {
        let this = self.await?;

        if this.has_top_level_await {
            return Ok(BoolVc::cell(this.has_top_level_await));
        }

        let references_async = this
            .references
            .iter()
            .map(|r| async { anyhow::Ok(*r.is_async().await?) })
            .try_join()
            .await?;

        Ok(BoolVc::cell(references_async.contains(&true)))
    }

    #[turbo_tasks::function]
    pub(crate) async fn module_options(self) -> Result<OptionAsyncModuleOptionsVc> {
        if !*self.is_async().await? {
            return Ok(OptionAsyncModuleOptionsVc::cell(None));
        }

        Ok(OptionAsyncModuleOptionsVc::cell(Some(AsyncModuleOptions {
            has_top_level_await: self.await?.has_top_level_await,
        })))
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for AsyncModule {
    #[turbo_tasks::function]
    async fn code_generation(
        self_vc: AsyncModuleVc,
        _context: EcmascriptChunkingContextVc,
    ) -> Result<CodeGenerationVc> {
        let this = self_vc.await?;
        let mut visitors = Vec::new();

        if *self_vc.is_async().await? {
            let reference_idents: Vec<Option<String>> = this
                .references
                .iter()
                .map(|r| async {
                    let referenced_asset = r.get_referenced_asset().await?;
                    let ident = referenced_asset.get_ident().await?;
                    anyhow::Ok(ident)
                })
                .try_join()
                .await?;

            let reference_idents = reference_idents
                .into_iter()
                .flatten()
                .collect::<IndexSet<_>>();

            if !reference_idents.is_empty() {
                visitors.push(create_visitor!(visit_mut_program(program: &mut Program) {
                    add_async_dependency_handler(program, &reference_idents);
                }));
            }
        }

        Ok(CodeGeneration { visitors }.into())
    }
}

fn add_async_dependency_handler(program: &mut Program, idents: &IndexSet<String>) {
    let idents = idents
        .iter()
        .map(|ident| Ident::new(ident.clone().into(), DUMMY_SP))
        .collect::<Vec<_>>();

    let stmt = quote!(
        "var __turbopack_async_dependencies__ = __turbopack_handle_async_dependencies__($deps);"
            as Stmt,
        deps: Expr = Expr::Array(ArrayLit {
            span: DUMMY_SP,
            elems: idents
                .iter()
                .map(|ident| { Some(Expr::Ident(ident.clone()).into()) })
                .collect(),
        }),
    );

    insert_hoisted_stmt(program, stmt);

    let stmt = quote!(
        "($deps = __turbopack_async_dependencies__.then ? (await \
         __turbopack_async_dependencies__)() : __turbopack_async_dependencies__);" as Stmt,
        deps: Pat = Pat::Array(ArrayPat {
            span: DUMMY_SP,
            elems: idents
                .into_iter()
                .map(|ident| { Some(ident.into()) })
                .collect(),
            optional: false,
            type_ann: None,
        }),
    );

    insert_hoisted_stmt(program, stmt);
}
