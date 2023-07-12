use anyhow::Result;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use swc_core::{
    common::DUMMY_SP,
    ecma::ast::{ArrayLit, ArrayPat, Expr, Ident, Pat, Program},
    quote,
};
use turbo_tasks::{
    primitives::{BoolVc, BoolVcsVc},
    trace::TraceRawVcs,
    TryFlatJoinIterExt, Value,
};
use turbopack_core::chunk::availability_info::AvailabilityInfo;

use crate::{
    chunk::{
        esm_scope::{EsmScopeSccVc, EsmScopeVc},
        EcmascriptChunkPlaceable, EcmascriptChunkPlaceableVc, EcmascriptChunkingContextVc,
    },
    code_gen::{CodeGenerateableWithAvailabilityInfo, CodeGeneration, CodeGenerationVc},
    create_visitor,
    references::esm::{base::insert_hoisted_stmt, EsmAssetReferenceVc},
    CodeGenerateableWithAvailabilityInfoVc, EcmascriptModuleAssetVc,
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
    pub(super) module: EcmascriptModuleAssetVc,
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
    pub(crate) async fn is_async(
        self,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<BoolVc> {
        Ok(BoolVc::cell(
            self.module_options(availability_info).await?.is_some(),
        ))
    }

    #[turbo_tasks::function]
    pub(crate) async fn module_options(
        self,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<OptionAsyncModuleOptionsVc> {
        if let Some(async_module) = &*self.await? {
            return Ok(async_module.module_options(availability_info));
        }

        Ok(OptionAsyncModuleOptionsVc::none())
    }
}

#[turbo_tasks::value]
pub struct AsyncModuleScc {
    scc: EsmScopeSccVc,
    scope: EsmScopeVc,
}

#[turbo_tasks::value(transparent)]
pub struct OptionAsyncModuleScc(Option<AsyncModuleSccVc>);

#[turbo_tasks::function]
async fn is_placeable_self_async(placeable: EcmascriptChunkPlaceableVc) -> Result<BoolVc> {
    let Some(async_module) = &*placeable.get_async_module().await? else {
        return Ok(BoolVc::cell(false));
    };

    Ok(async_module.is_self_async())
}

#[turbo_tasks::value_impl]
impl AsyncModuleSccVc {
    #[turbo_tasks::function]
    fn new(scc: EsmScopeSccVc, scope: EsmScopeVc) -> Self {
        Self::cell(AsyncModuleScc { scc, scope })
    }

    #[turbo_tasks::function]
    pub(crate) async fn is_async(self) -> Result<BoolVc> {
        let this = self.await?;

        let mut bools = Vec::new();

        for placeable in &*this.scc.await? {
            bools.push(is_placeable_self_async(*placeable));
        }

        for scc in &*this.scope.get_scc_children(this.scc).await? {
            // Because we generated SCCs there can be no loops in the children, so calling
            // recursively is fine.
            bools.push(AsyncModuleSccVc::new(*scc, this.scope).is_async());
        }

        Ok(BoolVcsVc::cell(bools).any())
    }
}

#[turbo_tasks::value(transparent)]
pub struct AsyncModuleIdents(IndexSet<String>);

#[turbo_tasks::value_impl]
impl AsyncModuleVc {
    #[turbo_tasks::function]
    pub(crate) async fn get_async_idents(
        self,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<AsyncModuleIdentsVc> {
        let this = self.await?;

        let reference_idents = this
            .references
            .iter()
            .map(|r| async {
                let referenced_asset = r.get_referenced_asset().await?;
                let ident = if *r.is_async(availability_info).await? {
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
    pub(crate) async fn has_top_level_await(self) -> Result<BoolVc> {
        Ok(BoolVc::cell(self.await?.has_top_level_await))
    }

    #[turbo_tasks::function]
    pub(crate) async fn is_self_async(self) -> Result<BoolVc> {
        let this = self.await?;

        if this.has_top_level_await {
            return Ok(BoolVc::cell(true));
        }

        let bools = BoolVcsVc::cell(
            this.references
                .iter()
                .map(|r| r.is_external_esm())
                .collect(),
        );

        Ok(bools.any())
    }

    #[turbo_tasks::function]
    async fn get_scc(
        self,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<OptionAsyncModuleSccVc> {
        let this = self.await?;

        let scope = EsmScopeVc::new(availability_info);
        let Some(scc) = &*scope
            .get_scc(this.module.as_ecmascript_chunk_placeable())
            .await?
        else {
            // I'm not sure if this should be possible.
            return Ok(OptionAsyncModuleSccVc::cell(None));
        };

        let scc = AsyncModuleSccVc::new(*scc, scope);

        Ok(OptionAsyncModuleSccVc::cell(Some(scc)))
    }

    #[turbo_tasks::function]
    pub(crate) async fn is_async(
        self,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<BoolVc> {
        Ok(
            if let Some(scc) = &*self.get_scc(availability_info).await? {
                scc.is_async()
            } else {
                self.is_self_async()
            },
        )
    }

    #[turbo_tasks::function]
    pub(crate) async fn module_options(
        self,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<OptionAsyncModuleOptionsVc> {
        if !*self.is_async(availability_info).await? {
            return Ok(OptionAsyncModuleOptionsVc::cell(None));
        }

        Ok(OptionAsyncModuleOptionsVc::cell(Some(AsyncModuleOptions {
            has_top_level_await: self.await?.has_top_level_await,
        })))
    }
}

#[turbo_tasks::value_impl]
impl CodeGenerateableWithAvailabilityInfo for AsyncModule {
    #[turbo_tasks::function]
    async fn code_generation(
        self_vc: AsyncModuleVc,
        _context: EcmascriptChunkingContextVc,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<CodeGenerationVc> {
        let mut visitors = Vec::new();

        let async_idents = self_vc.get_async_idents(availability_info).await?;

        if !async_idents.is_empty() {
            visitors.push(create_visitor!(visit_mut_program(program: &mut Program) {
                add_async_dependency_handler(program, &async_idents);
            }));
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
