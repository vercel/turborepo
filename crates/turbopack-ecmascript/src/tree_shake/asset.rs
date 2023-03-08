use anyhow::{anyhow, Result};
use turbo_tasks::{primitives::StringVc, Value};
use turbopack_core::{
    asset::{Asset, AssetContentVc, AssetVc},
    chunk::{ChunkVc, ChunkableAsset, ChunkableAssetVc, ChunkingContextVc},
    ident::{AssetIdent, AssetIdentVc},
    reference::{AssetReferencesVc, SingleAssetReferenceVc},
    resolve::{origin::ResolveOrigin, ModulePart, ModulePartVc},
};

use super::{
    chunk_item::{EcmascriptModulePartChunkItem, EcmascriptModulePartChunkItemVc},
    split, Key, SplitResultVc,
};
use crate::{
    chunk::{
        EcmascriptChunkItemVc, EcmascriptChunkPlaceable, EcmascriptChunkPlaceableVc,
        EcmascriptChunkVc, EcmascriptExportsVc,
    },
    references::analyze_ecmascript_module,
    AnalyzeEcmascriptModuleResultVc, EcmascriptModuleAssetVc,
};

#[turbo_tasks::value]
pub struct EcmascriptModulePartAsset {
    full_module: EcmascriptModuleAssetVc,
    split_data: SplitResultVc,
    part_id: u32,
}

impl EcmascriptModulePartAssetVc {
    pub(super) fn new(data: EcmascriptModulePartAsset) -> Self {
        data.cell()
    }

    pub async fn from_split(module: EcmascriptModuleAssetVc, part: ModulePartVc) -> Result<Self> {
        let split_data = split(module.origin_path(), module.parse());
        let result = split_data.await?;
        let part = part.await?;

        let key = match &*part {
            ModulePart::ModuleEvaluation => Key::ModuleEvaluation,
            ModulePart::Export(export) => Key::Export(export.await?.to_string()),
        };

        let chunk_id = match result.data.get(&key) {
            Some(id) => *id,
            None => return Err(anyhow!("could not find part id for module part {:?}", key)),
        };

        Ok(EcmascriptModulePartAsset {
            full_module: module,
            part_id: chunk_id,
            split_data,
        }
        .cell())
    }
}

#[turbo_tasks::value_impl]
impl Asset for EcmascriptModulePartAsset {
    #[turbo_tasks::function]
    fn content(&self) -> AssetContentVc {
        todo!()
    }

    #[turbo_tasks::function]
    async fn references(&self) -> Result<AssetReferencesVc> {
        let split_data = self.split_data.await?;
        let deps = match split_data.deps.get(&self.part_id) {
            Some(v) => v,
            None => return Ok(self.full_module.references()),
        };

        let mut assets = deps
            .iter()
            .map(|&part_id| {
                SingleAssetReferenceVc::new(
                    EcmascriptModulePartAssetVc::new(EcmascriptModulePartAsset {
                        full_module: self.full_module,
                        part_id,
                        split_data: self.split_data,
                    })
                    .as_asset(),
                    StringVc::cell("ecmascript module part".to_string()),
                )
                .as_asset_reference()
            })
            .collect::<Vec<_>>();

        let external = self.full_module.references().await?;

        assets.extend(external.iter().cloned());

        Ok(AssetReferencesVc::cell(assets))
    }

    #[turbo_tasks::function]
    async fn ident(&self) -> Result<AssetIdentVc> {
        let inner = self.full_module.ident();

        Ok(inner.with_part(self.part_id))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for EcmascriptModulePartAsset {
    #[turbo_tasks::function]
    async fn as_chunk_item(
        self_vc: EcmascriptModulePartAssetVc,
        context: ChunkingContextVc,
    ) -> Result<EcmascriptChunkItemVc> {
        let s = self_vc.await?;

        Ok(
            EcmascriptModulePartChunkItemVc::new(EcmascriptModulePartChunkItem {
                module: self_vc,
                context,
                chunk_id: s.part_id,
                full_module: s.full_module,
                split_data: s.split_data,
            })
            .into(),
        )
    }

    #[turbo_tasks::function]
    async fn get_exports(self_vc: EcmascriptModuleAssetVc) -> Result<EcmascriptExportsVc> {
        Ok(self_vc.analyze().await?.exports)
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAsset for EcmascriptModulePartAsset {
    #[turbo_tasks::function]
    async fn as_chunk(self_vc: EcmascriptModulePartAssetVc, context: ChunkingContextVc) -> ChunkVc {
        EcmascriptChunkVc::new(context, self_vc.as_ecmascript_chunk_placeable()).into()
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptModulePartAssetVc {
    #[turbo_tasks::function]
    pub(super) async fn analyze(self) -> Result<AnalyzeEcmascriptModuleResultVc> {
        let part = self.await?;
        let this = part.full_module.await?;
        Ok(analyze_ecmascript_module(
            this.source,
            part.full_module.as_resolve_origin(),
            Value::new(this.ty),
            this.transforms,
            this.compile_time_info,
            Some(part.part_id),
        ))
    }
}
