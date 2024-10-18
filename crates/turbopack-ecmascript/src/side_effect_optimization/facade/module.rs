use std::collections::BTreeMap;

use anyhow::{bail, Result};
use turbo_tasks::Vc;
use turbo_tasks_fs::{glob::Glob, File, FileContent};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::{ChunkableModule, ChunkingContext, EvaluatableAsset},
    ident::AssetIdent,
    module::Module,
    reference::ModuleReferences,
    resolve::ModulePart,
};

use super::chunk_item::EcmascriptModuleFacadeChunkItem;
use crate::{
    chunk::{EcmascriptChunkPlaceable, EcmascriptExports},
    references::{
        async_module::{AsyncModule, OptionAsyncModule},
        esm::{EsmExport, EsmExports},
    },
    side_effect_optimization::reference::EcmascriptModulePartReference,
    EcmascriptModuleAsset,
};

/// A module derived from an original ecmascript module that only contains all
/// the reexports from that module and also reexports the locals from
/// [EcmascriptModuleLocalsModule]. It allows to follow
#[turbo_tasks::value]
pub struct EcmascriptModuleFacadeModule {
    pub module: Vc<Box<dyn EcmascriptChunkPlaceable>>,
    pub ty: Vc<ModulePart>,
}

#[turbo_tasks::value_impl]
impl EcmascriptModuleFacadeModule {
    #[turbo_tasks::function]
    pub fn new(module: Vc<Box<dyn EcmascriptChunkPlaceable>>, ty: Vc<ModulePart>) -> Vc<Self> {
        EcmascriptModuleFacadeModule { module, ty }.cell()
    }

    #[turbo_tasks::function]
    pub async fn async_module(self: Vc<Self>) -> Result<Vc<AsyncModule>> {
        let (import_externals, has_top_level_await) =
            if let Some(async_module) = *self.await?.module.get_async_module().await? {
                (
                    async_module.await?.import_externals,
                    async_module.await?.has_top_level_await,
                )
            } else {
                (false, false)
            };
        Ok(AsyncModule {
            placeable: Vc::upcast(self),
            has_top_level_await,
            import_externals,
        }
        .cell())
    }
}

#[turbo_tasks::value_impl]
impl Module for EcmascriptModuleFacadeModule {
    #[turbo_tasks::function]
    async fn ident(&self) -> Result<Vc<AssetIdent>> {
        let inner = self.module.ident();

        Ok(inner.with_part(self.ty))
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<Vc<ModuleReferences>> {
        let references = match &*self.ty.await? {
            ModulePart::Evaluation => {
                let Some(module) =
                    Vc::try_resolve_downcast_type::<EcmascriptModuleAsset>(self.module).await?
                else {
                    bail!(
                        "Expected EcmascriptModuleAsset for a EcmascriptModuleFacadeModule with \
                         ModulePart::Evaluation"
                    );
                };
                let result = module.failsafe_analyze().await?;
                let references = result.evaluation_references;
                let mut references = references.await?.clone_value();
                references.push(Vc::upcast(EcmascriptModulePartReference::new_part(
                    self.module,
                    ModulePart::locals(),
                )));
                references
            }
            ModulePart::Exports => {
                let Some(module) =
                    Vc::try_resolve_downcast_type::<EcmascriptModuleAsset>(self.module).await?
                else {
                    bail!(
                        "Expected EcmascriptModuleAsset for a EcmascriptModuleFacadeModule with \
                         ModulePart::Evaluation"
                    );
                };
                let result = module.failsafe_analyze().await?;
                let references = result.reexport_references;
                let mut references = references.await?.clone_value();
                references.push(Vc::upcast(EcmascriptModulePartReference::new_part(
                    self.module,
                    ModulePart::locals(),
                )));
                references
            }
            ModulePart::Facade => {
                vec![
                    Vc::upcast(EcmascriptModulePartReference::new_part(
                        self.module,
                        ModulePart::evaluation(),
                    )),
                    Vc::upcast(EcmascriptModulePartReference::new_part(
                        self.module,
                        ModulePart::exports(),
                    )),
                ]
            }
            ModulePart::RenamedNamespace { .. } => {
                vec![Vc::upcast(EcmascriptModulePartReference::new(self.module))]
            }
            ModulePart::RenamedExport { .. } => {
                vec![Vc::upcast(EcmascriptModulePartReference::new(self.module))]
            }
            _ => {
                bail!("Unexpected ModulePart for EcmascriptModuleFacadeModule");
            }
        };
        Ok(Vc::cell(references))
    }
}

#[turbo_tasks::value_impl]
impl Asset for EcmascriptModuleFacadeModule {
    #[turbo_tasks::function]
    async fn content(&self) -> Result<Vc<AssetContent>> {
        let f = File::from("");

        Ok(AssetContent::file(FileContent::Content(f).cell()))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for EcmascriptModuleFacadeModule {
    #[turbo_tasks::function]
    async fn get_exports(&self) -> Result<Vc<EcmascriptExports>> {
        let mut exports = BTreeMap::new();
        let mut star_exports = Vec::new();

        match &*self.ty.await? {
            ModulePart::Exports => {
                let EcmascriptExports::EsmExports(esm_exports) = *self.module.get_exports().await?
                else {
                    bail!(
                        "EcmascriptModuleFacadeModule must only be used on modules with EsmExports"
                    );
                };
                let esm_exports = esm_exports.await?;
                for (name, export) in &esm_exports.exports {
                    let name = name.clone();
                    match export {
                        EsmExport::LocalBinding(_, mutable) => {
                            exports.insert(
                                name.clone(),
                                EsmExport::ImportedBinding(
                                    Vc::upcast(EcmascriptModulePartReference::new_part(
                                        self.module,
                                        ModulePart::locals(),
                                    )),
                                    name,
                                    *mutable,
                                ),
                            );
                        }
                        EsmExport::ImportedNamespace(reference) => {
                            exports.insert(name, EsmExport::ImportedNamespace(*reference));
                        }
                        EsmExport::ImportedBinding(reference, imported_name, mutable) => {
                            exports.insert(
                                name,
                                EsmExport::ImportedBinding(
                                    *reference,
                                    imported_name.clone(),
                                    *mutable,
                                ),
                            );
                        }
                        EsmExport::Error => {
                            exports.insert(name, EsmExport::Error);
                        }
                    }
                }
                star_exports.extend(esm_exports.star_exports.iter().copied());
            }
            ModulePart::Facade => {
                // Reexport everything from the reexports module
                // (including default export if any)
                let EcmascriptExports::EsmExports(esm_exports) = *self.module.get_exports().await?
                else {
                    bail!(
                        "EcmascriptModuleFacadeModule must only be used on modules with EsmExports"
                    );
                };
                let esm_exports = esm_exports.await?;
                if esm_exports.exports.keys().any(|name| name == "default") {
                    exports.insert(
                        "default".to_string(),
                        EsmExport::ImportedBinding(
                            Vc::upcast(EcmascriptModulePartReference::new_part(
                                self.module,
                                ModulePart::exports(),
                            )),
                            "default".to_string(),
                            false,
                        ),
                    );
                }
                star_exports.push(Vc::upcast(EcmascriptModulePartReference::new_part(
                    self.module,
                    ModulePart::exports(),
                )));
            }
            ModulePart::RenamedExport {
                original_export,
                export,
            } => {
                let original_export = original_export.await?;
                exports.insert(
                    export.await?.clone_value(),
                    EsmExport::ImportedBinding(
                        Vc::upcast(EcmascriptModulePartReference::new(self.module)),
                        original_export.clone_value(),
                        false,
                    ),
                );
            }
            ModulePart::RenamedNamespace { export } => {
                exports.insert(
                    export.await?.clone_value(),
                    EsmExport::ImportedNamespace(Vc::upcast(EcmascriptModulePartReference::new(
                        self.module,
                    ))),
                );
            }
            ModulePart::Evaluation => {
                // no exports
            }
            _ => bail!("Unexpected ModulePart for EcmascriptModuleFacadeModule"),
        }

        let exports = EsmExports {
            exports,
            star_exports,
        }
        .cell();
        Ok(EcmascriptExports::EsmExports(exports).cell())
    }

    #[turbo_tasks::function]
    async fn is_marked_as_side_effect_free(
        &self,
        side_effect_free_packages: Vc<Glob>,
    ) -> Result<Vc<bool>> {
        Ok(match *self.ty.await? {
            ModulePart::Evaluation | ModulePart::Facade => self
                .module
                .is_marked_as_side_effect_free(side_effect_free_packages),
            ModulePart::Exports
            | ModulePart::RenamedExport { .. }
            | ModulePart::RenamedNamespace { .. } => Vc::cell(true),
            _ => bail!("Unexpected ModulePart for EcmascriptModuleFacadeModule"),
        })
    }

    #[turbo_tasks::function]
    fn get_async_module(self: Vc<Self>) -> Vc<OptionAsyncModule> {
        Vc::cell(Some(self.async_module()))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModule for EcmascriptModuleFacadeModule {
    #[turbo_tasks::function]
    async fn as_chunk_item(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<Box<dyn turbopack_core::chunk::ChunkItem>>> {
        Ok(Vc::upcast(
            EcmascriptModuleFacadeChunkItem {
                module: self,
                chunking_context,
            }
            .cell(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl EvaluatableAsset for EcmascriptModuleFacadeModule {}
