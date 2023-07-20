use turbo_tasks::{ValueToString, Vc};

use crate::{
    asset::{Asset, AssetContent},
    ident::AssetIdent,
    module::Module,
    reference::{AssetReference, ModuleReference},
    resolve::ModuleResolveResult,
    source::Source,
};

/// A module where source code doesn't need to be parsed but can be usd as is.
/// This module has no references to other modules.
#[turbo_tasks::value]
pub struct RawModule {
    source: Vc<Box<dyn Source>>,
}

#[turbo_tasks::value_impl]
impl Module for RawModule {
    #[turbo_tasks::function]
    fn ident(&self) -> Vc<AssetIdent> {
        self.source.ident()
    }
}

#[turbo_tasks::value_impl]
impl Asset for RawModule {
    #[turbo_tasks::function]
    fn content(&self) -> Vc<AssetContent> {
        self.source.content()
    }
}

#[turbo_tasks::value_impl]
impl RawModule {
    #[turbo_tasks::function]
    pub fn new(source: Vc<Box<dyn Source>>) -> Vc<RawModule> {
        RawModule { source }.cell()
    }
}

#[turbo_tasks::value]
pub struct RawModuleReference {
    reference: Vc<Box<dyn AssetReference>>,
}

#[turbo_tasks::value_impl]
impl RawModuleReference {
    #[turbo_tasks::function]
    pub fn new(reference: Vc<Box<dyn AssetReference>>) -> Vc<RawModuleReference> {
        RawModuleReference { reference }.cell()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for RawModuleReference {
    #[turbo_tasks::function]
    fn to_string(&self) -> Vc<String> {
        self.reference.to_string()
    }
}

#[turbo_tasks::value_impl]
impl ModuleReference for RawModuleReference {
    #[turbo_tasks::function]
    fn resolve_reference(&self) -> Vc<ModuleResolveResult> {
        self.reference.resolve_reference().as_raw_module_result()
    }
}
