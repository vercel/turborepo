use anyhow::Result;
use turbo_tasks::{ValueToString, Vc};
use turbopack_core::{reference::ModuleReference, resolve::ModuleResolveResult};

use super::module::{EcmascriptModuleReexportsPartModule, EcmascriptModuleReexportsPartModuleType};
use crate::EcmascriptModuleAsset;

#[turbo_tasks::value]
pub struct EcmascriptModuleReexportsPartModuleLocalsReference {
    pub module: Vc<EcmascriptModuleAsset>,
}

#[turbo_tasks::value_impl]
impl EcmascriptModuleReexportsPartModuleLocalsReference {
    #[turbo_tasks::function]
    pub fn new(module: Vc<EcmascriptModuleAsset>) -> Vc<Self> {
        EcmascriptModuleReexportsPartModuleLocalsReference { module }.cell()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptModuleReexportsPartModuleLocalsReference {
    #[turbo_tasks::function]
    fn to_string(&self) -> Vc<String> {
        Vc::cell("locals".to_string())
    }
}

#[turbo_tasks::value_impl]
impl ModuleReference for EcmascriptModuleReexportsPartModuleLocalsReference {
    #[turbo_tasks::function]
    async fn resolve_reference(self: Vc<Self>) -> Result<Vc<ModuleResolveResult>> {
        let locals_module = EcmascriptModuleReexportsPartModule::new(
            self.await?.module,
            EcmascriptModuleReexportsPartModuleType::Locals,
        );
        Ok(ModuleResolveResult::module(Vc::upcast(locals_module)).cell())
    }
}
