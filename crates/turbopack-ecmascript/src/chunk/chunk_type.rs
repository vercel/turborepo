use anyhow::{bail, Result};
use turbo_tasks::{TryJoinIterExt, ValueDefault, Vc};
use turbopack_core::{
    chunk::{Chunk, ChunkItems, ChunkType, ChunkingContext},
    ident::AssetIdent,
    module::Module,
    output::OutputAssets,
};

use super::{
    EcmascriptChunk, EcmascriptChunkContent, EcmascriptChunkItem, EcmascriptChunkingContext,
};

#[derive(Default)]
#[turbo_tasks::value]
pub struct EcmascriptChunkType {}

#[turbo_tasks::value_impl]
impl ChunkType for EcmascriptChunkType {
    #[turbo_tasks::function]
    async fn chunk(
        &self,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
        ident: Vc<AssetIdent>,
        chunk_items: Vc<ChunkItems>,
        referenced_output_assets: Vc<OutputAssets>,
        chunk_group_root: Option<Vc<Box<dyn Module>>>,
    ) -> Result<Vc<Box<dyn Chunk>>> {
        let Some(chunking_context) =
            Vc::try_resolve_downcast::<Box<dyn EcmascriptChunkingContext>>(chunking_context)
                .await?
        else {
            bail!("Ecmascript chunking context not found");
        };
        let content = EcmascriptChunkContent {
            chunk_items: chunk_items
                .await?
                .iter()
                .map(|&chunk_item| async move {
                    let Some(chunk_item) =
                        Vc::try_resolve_downcast::<Box<dyn EcmascriptChunkItem>>(chunk_item)
                            .await?
                    else {
                        bail!(
                            "Chunk item is not an ecmascript chunk item but reporting chunk type \
                             ecmascript"
                        );
                    };
                    Ok(chunk_item)
                })
                .try_join()
                .await?,
            referenced_output_assets: referenced_output_assets.await?.clone_value(),
            chunk_group_root,
        }
        .cell();
        Ok(Vc::upcast(EcmascriptChunk::new(
            chunking_context,
            ident,
            content,
        )))
    }
}

#[turbo_tasks::value_impl]
impl ValueDefault for EcmascriptChunkType {
    #[turbo_tasks::function]
    fn value_default() -> Vc<Self> {
        Self::default().cell()
    }
}
