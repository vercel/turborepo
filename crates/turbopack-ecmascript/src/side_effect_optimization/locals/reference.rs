use anyhow::Result;
use turbo_tasks::{ValueToString, Vc};
use turbopack_core::{
    chunk::ChunkableModuleReference, reference::ModuleReference, resolve::ModuleResolveResult,
};

use super::module::EcmascriptModuleLocalsModule;
use crate::EcmascriptModuleAsset;

/// A reference to the [EcmascriptModuleLocalsModule] variant of an original
/// [EcmascriptModuleAsset].
#[turbo_tasks::value]
pub struct EcmascriptModuleLocalsReference {
    pub module: Vc<EcmascriptModuleAsset>,
}

#[turbo_tasks::value_impl]
impl EcmascriptModuleLocalsReference {
    #[turbo_tasks::function]
    pub fn new(module: Vc<EcmascriptModuleAsset>) -> Vc<Self> {
        EcmascriptModuleLocalsReference { module }.cell()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptModuleLocalsReference {
    #[turbo_tasks::function]
    fn to_string(&self) -> Vc<String> {
        Vc::cell("locals".to_string())
    }
}

#[turbo_tasks::value_impl]
impl ModuleReference for EcmascriptModuleLocalsReference {
    #[turbo_tasks::function]
    async fn resolve_reference(self: Vc<Self>) -> Result<Vc<ModuleResolveResult>> {
        let locals_module = EcmascriptModuleLocalsModule::new(self.await?.module);
        Ok(ModuleResolveResult::module(Vc::upcast(locals_module)).cell())
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModuleReference for EcmascriptModuleLocalsReference {}
