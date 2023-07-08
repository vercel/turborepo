use std::future::IntoFuture;

use anyhow::Result;
use futures::{stream::FuturesOrdered, TryStreamExt};
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use swc_core::{
    common::DUMMY_SP,
    ecma::ast::{ArrayLit, ArrayPat, Expr, Ident, Pat, Program},
    quote,
};
use turbo_tasks::{primitives::BoolVc, trace::TraceRawVcs, TryFlatJoinIterExt};

use crate::{
    chunk::{EcmascriptChunkPlaceable, EcmascriptChunkingContextVc},
    code_gen::{CodeGenerateable, CodeGeneration, CodeGenerationVc},
    create_visitor,
    references::esm::{
        base::{insert_hoisted_stmt, ReferencedAsset},
        EsmAssetReferenceVc,
    },
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
    pub(crate) fn none() -> Self {
        Self::cell(None)
    }

    #[turbo_tasks::function]
    pub(crate) async fn is_async(self) -> Result<BoolVc> {
        Ok(BoolVc::cell(self.await?.is_some()))
    }
}

#[turbo_tasks::value(shared)]
pub struct AsyncModule {
    pub(super) references: IndexSet<EsmAssetReferenceVc>,
    pub(super) has_top_level_await: bool,
}

#[turbo_tasks::value(transparent)]
pub struct AsyncModules(IndexSet<AsyncModuleVc>);

#[turbo_tasks::value(transparent)]
pub struct OptionAsyncModule(Option<AsyncModuleVc>);

#[turbo_tasks::value_impl]
impl OptionAsyncModuleVc {
    #[turbo_tasks::function]
    pub(crate) fn none() -> Self {
        Self::cell(None)
    }

    #[turbo_tasks::function]
    pub(crate) async fn is_async(self) -> Result<BoolVc> {
        Ok(BoolVc::cell(self.module_options().await?.is_some()))
    }

    #[turbo_tasks::function]
    pub(crate) async fn module_options(self) -> Result<OptionAsyncModuleOptionsVc> {
        if let Some(async_module) = &*self.await? {
            return Ok(async_module.module_options());
        }

        Ok(OptionAsyncModuleOptionsVc::none())
    }
}

#[turbo_tasks::value(transparent)]
pub struct AsyncModuleIdents(IndexSet<String>);

#[turbo_tasks::value_impl]
impl AsyncModuleVc {
    /// Collects all [AsyncModuleVc]s from the references and returns them
    /// after resolving.
    #[turbo_tasks::function]
    pub(crate) async fn collect_direct_async_module_children(self) -> Result<AsyncModulesVc> {
        let this = self.await?;

        let async_modules = this
            .references
            .iter()
            .map(|r| async {
                let referenced_asset = r.get_referenced_asset().await?;
                let ReferencedAsset::Some(placeable) = &*referenced_asset else {
                    return anyhow::Ok(None);
                };

                let Some(async_module) = &*placeable.get_async_module().await? else {
                    return anyhow::Ok(None);
                };

                let resolved = async_module.resolve().await?;
                if resolved == self {
                    return anyhow::Ok(None);
                };

                anyhow::Ok(Some(resolved))
            })
            .try_flat_join()
            .await?;

        Ok(AsyncModulesVc::cell(IndexSet::from_iter(async_modules)))
    }

    /// Collects all [AsyncModuleVc]s referenced including the current
    /// [AsyncModuleVc].
    #[turbo_tasks::function]
    pub(crate) async fn collect_all_async_modules(self) -> Result<AsyncModulesVc> {
        let mut futures = FuturesOrdered::new();
        futures.push_back(self.collect_direct_async_module_children().into_future());

        let mut async_modules = IndexSet::from([self]);
        while let Some(modules) = futures.try_next().await? {
            for async_module in modules.iter().copied() {
                if async_modules.insert(async_module) {
                    futures.push_back(
                        async_module
                            .collect_direct_async_module_children()
                            .into_future(),
                    );
                }
            }
        }

        Ok(AsyncModulesVc::cell(async_modules))
    }

    #[turbo_tasks::function]
    pub(crate) async fn get_async_idents(self) -> Result<AsyncModuleIdentsVc> {
        let this = self.await?;

        let reference_idents = this
            .references
            .iter()
            .map(|r| async {
                let referenced_asset = r.get_referenced_asset().await?;
                let ident = if *r.is_async(true).await? {
                    referenced_asset.get_ident().await?
                } else {
                    None
                };
                anyhow::Ok(ident)
            })
            .try_flat_join()
            .await?;

        Ok(AsyncModuleIdentsVc::cell(IndexSet::from_iter(
            reference_idents,
        )))
    }

    #[turbo_tasks::function]
    pub(crate) async fn is_self_async(self) -> Result<BoolVc> {
        let this = self.await?;

        if this.has_top_level_await {
            return Ok(BoolVc::cell(true));
        }

        let references = this
            .references
            .iter()
            .map(|r| async { anyhow::Ok((*r.is_async(false).await?).then_some(())) })
            .try_flat_join()
            .await?;

        Ok(BoolVc::cell(!references.is_empty()))
    }

    #[turbo_tasks::function]
    pub(crate) async fn is_async(self) -> Result<BoolVc> {
        if *self.is_self_async().await? {
            return Ok(BoolVc::cell(true));
        }

        let async_modules = self
            .collect_all_async_modules()
            .await?
            .iter()
            .map(|a| async {
                anyhow::Ok(if *a.is_self_async().await? {
                    Some(*a)
                } else {
                    None
                })
            })
            .try_flat_join()
            .await?;

        Ok(BoolVc::cell(!async_modules.is_empty()))
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
        let mut visitors = Vec::new();

        if *self_vc.is_async().await? {
            let async_idents = self_vc.get_async_idents().await?;

            if !async_idents.is_empty() {
                visitors.push(create_visitor!(visit_mut_program(program: &mut Program) {
                    add_async_dependency_handler(program, &async_idents);
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
