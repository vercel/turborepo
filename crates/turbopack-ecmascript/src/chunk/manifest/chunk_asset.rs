use anyhow::Result;
use turbo_tasks::primitives::StringVc;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::{
    asset::{Asset, AssetContentVc, AssetVc},
    chunk::{
        available_assets::AvailableAssetsVc, ChunkGroupVc, ChunkReferenceVc, ChunkVc,
        ChunkableAsset, ChunkableAssetVc, ChunkingContext, ChunkingContextVc,
    },
    ident::AssetIdentVc,
    reference::AssetReferencesVc,
};

use super::chunk_item::ManifestChunkItem;
use crate::chunk::{
    item::EcmascriptChunkItemVc,
    placeable::{
        EcmascriptChunkPlaceable, EcmascriptChunkPlaceableVc, EcmascriptExports,
        EcmascriptExportsVc,
    },
    EcmascriptChunkVc,
};

#[turbo_tasks::function]
fn modifier() -> StringVc {
    StringVc::cell("manifest chunk".to_string())
}

#[turbo_tasks::function]
fn chunk_list_modifier() -> StringVc {
    StringVc::cell("chunks list".to_string())
}

/// The manifest chunk is deferred until requested by the manifest loader
/// item when the dynamic `import()` expression is reached. Its responsibility
/// is to generate a Promise that will resolve only after all the necessary
/// chunks needed by the dynamic import are loaded by the client.
///
/// Splitting the dynamic import into a quickly generate-able manifest loader
/// item and a slow-to-generate manifest chunk allows for faster incremental
/// compilation. The traversal won't be performed until the dynamic import is
/// actually reached, instead of eagerly as part of the chunk that the dynamic
/// import appears in.
#[turbo_tasks::value(shared)]
pub struct ManifestChunkAsset {
    pub asset: ChunkableAssetVc,
    pub chunking_context: ChunkingContextVc,
    pub available_assets: Option<AvailableAssetsVc>,
}

#[turbo_tasks::value_impl]
impl ManifestChunkAssetVc {
    #[turbo_tasks::function]
    pub fn new(
        asset: ChunkableAssetVc,
        chunking_context: ChunkingContextVc,
        available_assets: Option<AvailableAssetsVc>,
    ) -> Self {
        Self::cell(ManifestChunkAsset {
            asset,
            chunking_context,
            available_assets,
        })
    }

    #[turbo_tasks::function]
    pub(super) async fn chunk_group(self) -> Result<ChunkGroupVc> {
        let this = self.await?;
        Ok(ChunkGroupVc::from_asset(
            this.asset,
            this.chunking_context,
            this.available_assets,
            Some(this.asset.as_asset()),
        ))
    }

    #[turbo_tasks::function]
    pub(super) async fn chunk_list_path(self) -> Result<FileSystemPathVc> {
        let this = &*self.await?;
        Ok(this.chunking_context.chunk_list_path(self.ident()))
    }

    #[turbo_tasks::function]
    pub async fn manifest_chunk(self) -> Result<ChunkVc> {
        let this = self.await?;
        Ok(self.as_chunk(
            this.chunking_context,
            this.available_assets,
            Some(self.as_asset()),
        ))
    }

    #[turbo_tasks::function]
    async fn chunks_list_path(self) -> Result<FileSystemPathVc> {
        Ok(self
            .await?
            .chunking_context
            .chunk_path(self.ident().with_modifier(chunk_list_modifier()), ".json"))
    }
}

#[turbo_tasks::value_impl]
impl Asset for ManifestChunkAsset {
    #[turbo_tasks::function]
    fn ident(&self) -> AssetIdentVc {
        self.asset.ident().with_modifier(modifier())
    }

    #[turbo_tasks::function]
    fn content(&self) -> AssetContentVc {
        todo!()
    }

    #[turbo_tasks::function]
    async fn references(self_vc: ManifestChunkAssetVc) -> Result<AssetReferencesVc> {
        let chunks = self_vc.chunk_group().chunks();

        Ok(AssetReferencesVc::cell(
            chunks
                .await?
                .iter()
                .copied()
                .map(ChunkReferenceVc::new)
                .map(Into::into)
                .collect(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl ChunkableAsset for ManifestChunkAsset {
    #[turbo_tasks::function]
    fn as_chunk(
        self_vc: ManifestChunkAssetVc,
        context: ChunkingContextVc,
        available_assets: Option<AvailableAssetsVc>,
        current_availability_root: Option<AssetVc>,
    ) -> ChunkVc {
        EcmascriptChunkVc::new(
            context,
            self_vc.into(),
            available_assets,
            current_availability_root,
        )
        .into()
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkPlaceable for ManifestChunkAsset {
    #[turbo_tasks::function]
    fn as_chunk_item(
        self_vc: ManifestChunkAssetVc,
        context: ChunkingContextVc,
    ) -> EcmascriptChunkItemVc {
        ManifestChunkItem {
            context,
            manifest: self_vc,
        }
        .cell()
        .into()
    }

    #[turbo_tasks::function]
    fn get_exports(&self) -> EcmascriptExportsVc {
        EcmascriptExports::Value.cell()
    }
}
