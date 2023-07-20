use anyhow::Result;
use turbo_tasks::{ValueToString, Vc};

use super::{
    utils::{children_from_output_assets, content_to_details},
    Introspectable, IntrospectableChildren,
};
use crate::{asset::Asset, output::OutputAsset};

#[turbo_tasks::value]
pub struct IntrospectableOutputAsset(Vc<Box<dyn OutputAsset>>);

#[turbo_tasks::value_impl]
impl IntrospectableOutputAsset {
    #[turbo_tasks::function]
    pub async fn new(asset: Vc<Box<dyn OutputAsset>>) -> Result<Vc<Box<dyn Introspectable>>> {
        Ok(Vc::try_resolve_sidecast::<Box<dyn Introspectable>>(asset)
            .await?
            .unwrap_or_else(|| Vc::upcast(IntrospectableOutputAsset(asset).cell())))
    }
}

#[turbo_tasks::function]
fn ty() -> Vc<String> {
    Vc::cell("output asset".to_string())
}

#[turbo_tasks::function]
fn reference_ty() -> Vc<String> {
    Vc::cell("reference".to_string())
}

#[turbo_tasks::function]
fn placed_or_parallel_reference_ty() -> Vc<String> {
    Vc::cell("placed/parallel reference".to_string())
}

#[turbo_tasks::function]
fn placed_reference_ty() -> Vc<String> {
    Vc::cell("placed reference".to_string())
}

#[turbo_tasks::function]
fn parallel_reference_ty() -> Vc<String> {
    Vc::cell("parallel reference".to_string())
}

#[turbo_tasks::function]
fn isolated_parallel_reference_ty() -> Vc<String> {
    Vc::cell("isolated parallel reference".to_string())
}

#[turbo_tasks::function]
fn async_reference_ty() -> Vc<String> {
    Vc::cell("async reference".to_string())
}

#[turbo_tasks::value_impl]
impl Introspectable for IntrospectableOutputAsset {
    #[turbo_tasks::function]
    fn ty(&self) -> Vc<String> {
        ty()
    }

    #[turbo_tasks::function]
    fn title(&self) -> Vc<String> {
        self.0.ident().to_string()
    }

    #[turbo_tasks::function]
    fn details(&self) -> Vc<String> {
        content_to_details(self.0.content())
    }

    #[turbo_tasks::function]
    fn children(&self) -> Vc<IntrospectableChildren> {
        children_from_output_assets(self.0.references())
    }
}
