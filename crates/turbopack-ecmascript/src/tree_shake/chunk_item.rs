use anyhow::Result;
use turbopack_core::{
    asset::Asset,
    chunk::{ChunkItem, ChunkItemVc, ChunkingContextVc, ModuleId, ModuleIdVc},
    ident::AssetIdentVc,
    reference::AssetReferencesVc,
    resolve::{origin::ResolveOrigin, ModulePart},
};

use super::{asset::EcmascriptModulePartAssetVc, part_of_module, split_module};
use crate::{
    chunk::{EcmascriptChunkItem, EcmascriptChunkItemContentVc, EcmascriptChunkItemVc},
    gen_content,
};

/// This is an implementation of [ChunkItem] for [EcmascriptModulePartAssetVc].
///
/// This is a pointer to a part of an ES module.
#[turbo_tasks::value(shared)]
pub struct EcmascriptModulePartChunkItem {
    pub(super) module: EcmascriptModulePartAssetVc,
    pub(super) context: ChunkingContextVc,
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for EcmascriptModulePartChunkItem {
    #[turbo_tasks::function]
    async fn content(&self) -> Result<EcmascriptChunkItemContentVc> {
        let module = self.module.await?;
        let split_data = split_module(module.full_module);
        let parsed = part_of_module(split_data, Some(module.part));

        Ok(gen_content(
            self.context,
            self.module.analyze(),
            parsed,
            module.full_module.ident(),
        ))
    }

    #[turbo_tasks::function]
    fn chunking_context(&self) -> ChunkingContextVc {
        self.context
    }

    #[turbo_tasks::function]
    async fn id(&self) -> Result<ModuleIdVc> {
        let module = self.module.await?;

        let part = module.part.await?;
        let module = module.full_module.origin_path().await?;

        match &*part {
            ModulePart::ModuleEvaluation => {
                Ok(ModuleId::String(format!("{} (ecmascript evaluation)", module.path)).into())
            }
            ModulePart::Export(name) => {
                let name = name.await?;
                Ok(
                    ModuleId::String(format!("{} (ecmascript export {})", module.path, name))
                        .into(),
                )
            }
            ModulePart::Internal(part_id) => Ok(ModuleId::String(format!(
                "{} (ecmascript part {})",
                module.path, part_id
            ))
            .into()),
        }
    }
}

#[turbo_tasks::value_impl]
impl ChunkItem for EcmascriptModulePartChunkItem {
    #[turbo_tasks::function]
    async fn references(&self) -> AssetReferencesVc {
        self.module.references()
    }

    #[turbo_tasks::function]
    async fn asset_ident(&self) -> Result<AssetIdentVc> {
        Ok(self.module.ident())
    }
}
