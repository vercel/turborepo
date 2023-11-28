use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use turbo_tasks::{debug::ValueDebugFormat, trace::TraceRawVcs, TaskInput, Vc};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::{ChunkableModule, ChunkingContext},
    ident::AssetIdent,
    module::Module,
    reference::ModuleReferences,
    resolve::ModulePart,
};

use super::{
    chunk_item::EcmascriptModuleReexportsPartChunkItem,
    reference::EcmascriptModuleReexportsPartModuleLocalsReference,
};
use crate::{
    chunk::{EcmascriptChunkPlaceable, EcmascriptChunkingContext, EcmascriptExports},
    references::esm::{EsmExport, EsmExports},
    EcmascriptModuleAsset,
};

#[derive(
    TaskInput,
    PartialEq,
    Eq,
    Hash,
    Clone,
    Copy,
    TraceRawVcs,
    ValueDebugFormat,
    Serialize,
    Deserialize,
    Debug,
)]
pub enum EcmascriptModuleReexportsPartModuleType {
    Locals,
    ReexportsFacade,
}

#[turbo_tasks::value]
pub struct EcmascriptModuleReexportsPartModule {
    pub module: Vc<EcmascriptModuleAsset>,
    pub ty: EcmascriptModuleReexportsPartModuleType,
}

#[turbo_tasks::value_impl]
impl EcmascriptModuleReexportsPartModule {
    #[turbo_tasks::function]
    pub fn new(
        module: Vc<EcmascriptModuleAsset>,
        ty: EcmascriptModuleReexportsPartModuleType,
    ) -> Vc<Self> {
        EcmascriptModuleReexportsPartModule { module, ty }.cell()
    }
}

#[turbo_tasks::value_impl]
impl Module for EcmascriptModuleReexportsPartModule {
    #[turbo_tasks::function]
    async fn ident(&self) -> Result<Vc<AssetIdent>> {
        let inner = self.module.ident();

        Ok(inner.with_part(match self.ty {
            EcmascriptModuleReexportsPartModuleType::Locals => ModulePart::locals(),
            EcmascriptModuleReexportsPartModuleType::ReexportsFacade => {
                ModulePart::reexports_facade()
            }
        }))
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<Vc<ModuleReferences>> {
        let result = self.module.failsafe_analyze().await?;
        Ok(match self.ty {
            EcmascriptModuleReexportsPartModuleType::Locals => result.references,
            EcmascriptModuleReexportsPartModuleType::ReexportsFacade => {
                let mut references = result.reexport_references.await?.clone_value();
                references.push(Vc::upcast(
                    EcmascriptModuleReexportsPartModuleLocalsReference::new(self.module),
                ));
                Vc::cell(references)
            }
        })
    }
}

#[turbo_tasks::value_impl]
impl Asset for EcmascriptModuleReexportsPartModule {
    #[turbo_tasks::function]
    fn content(&self) -> Vc<AssetContent> {
        // This is not reachable because EcmascriptModuleReexportsPartModule implements
        // ChunkableModule and ChunkableModule::as_chunk_item is called instead.
        todo!("EcmascriptModuleReexportsPartModule::content is not implemented")
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for EcmascriptModuleReexportsPartModule {
    #[turbo_tasks::function]
    async fn get_exports(&self) -> Result<Vc<EcmascriptExports>> {
        let result = self.module.failsafe_analyze().await?;
        let EcmascriptExports::EsmExports(exports) = *result.exports.await? else {
            bail!(
                "EcmascriptModuleReexportsPartModule must only be used on modules with EsmExports"
            );
        };
        let esm_exports = exports.await?;
        let mut exports = BTreeMap::new();
        let mut star_exports = Vec::new();
        match self.ty {
            EcmascriptModuleReexportsPartModuleType::Locals => {
                for (name, export) in &esm_exports.exports {
                    match export {
                        EsmExport::ImportedBinding(..) | EsmExport::ImportedNamespace(..) => {
                            // not included in locals module
                        }
                        EsmExport::LocalBinding(name) => {
                            exports.insert(name.clone(), EsmExport::LocalBinding(name.clone()));
                        }
                        EsmExport::Error => {
                            exports.insert(name.clone(), EsmExport::Error);
                        }
                    }
                }
            }
            EcmascriptModuleReexportsPartModuleType::ReexportsFacade => {
                for (name, export) in &esm_exports.exports {
                    let name = name.clone();
                    match export {
                        EsmExport::LocalBinding(local_name) => {
                            exports.insert(
                                name,
                                EsmExport::ImportedBinding(
                                    Vc::upcast(
                                        EcmascriptModuleReexportsPartModuleLocalsReference::new(
                                            self.module,
                                        ),
                                    ),
                                    local_name.clone(),
                                ),
                            );
                        }
                        EsmExport::ImportedNamespace(reference) => {
                            exports.insert(name, EsmExport::ImportedNamespace(*reference));
                        }
                        EsmExport::ImportedBinding(reference, imported_name) => {
                            exports.insert(
                                name,
                                EsmExport::ImportedBinding(*reference, imported_name.clone()),
                            );
                        }
                        EsmExport::Error => {
                            exports.insert(name, EsmExport::Error);
                        }
                    }
                }
                star_exports.extend(esm_exports.star_exports.iter().copied());
            }
        }
        let exports = EsmExports {
            exports,
            star_exports,
        }
        .cell();
        Ok(EcmascriptExports::EsmExports(exports).cell())
    }

    #[turbo_tasks::function]
    fn is_marked_as_side_effect_free(&self) -> Vc<bool> {
        match self.ty {
            EcmascriptModuleReexportsPartModuleType::Locals => {
                self.module.is_marked_as_side_effect_free()
            }
            EcmascriptModuleReexportsPartModuleType::ReexportsFacade => Vc::cell(true),
        }
    }
}

#[turbo_tasks::value_impl]
impl ChunkableModule for EcmascriptModuleReexportsPartModule {
    #[turbo_tasks::function]
    async fn as_chunk_item(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<Box<dyn turbopack_core::chunk::ChunkItem>>> {
        let chunking_context =
            Vc::try_resolve_downcast::<Box<dyn EcmascriptChunkingContext>>(chunking_context)
                .await?
                .context(
                    "chunking context must impl EcmascriptChunkingContext to use \
                     EcmascriptModuleReexportsPartModule",
                )?;
        Ok(Vc::upcast(
            EcmascriptModuleReexportsPartChunkItem {
                module: self,
                chunking_context,
            }
            .cell(),
        ))
    }
}
