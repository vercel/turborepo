use anyhow::{bail, Context, Result};
use turbo_tasks::{Value, ValueDefault, Vc};
use turbopack_core::{
    chunk::{availability_info::AvailabilityInfo, Chunk, ChunkItem, ChunkType},
    module::Module,
};

use super::{
    content::ecmascript_chunk_content, EcmascriptChunk, EcmascriptChunkPlaceable,
    EcmascriptChunkingContext,
};

#[derive(Default)]
#[turbo_tasks::value]
pub struct EcmascriptChunkType {}

#[turbo_tasks::value_impl]
impl ChunkType for EcmascriptChunkType {
    #[turbo_tasks::function]
    async fn as_chunk(
        &self,
        chunk_item: Vc<Box<dyn ChunkItem>>,
        availability_info: Value<AvailabilityInfo>,
    ) -> Result<Vc<Box<dyn Chunk>>> {
        let placeable =
            Vc::try_resolve_downcast::<Box<dyn EcmascriptChunkPlaceable>>(chunk_item.module())
                .await?
                .context(
                    "Module must implement EcmascriptChunkPlaceable to be used as a EcmaScript \
                     Chunk",
                )?;
        let Some(chunking_context) =
            Vc::try_resolve_downcast::<Box<dyn EcmascriptChunkingContext>>(
                chunk_item.chunking_context(),
            )
            .await?
        else {
            bail!("Ecmascript chunking context not found");
        };
        let ident = placeable.ident();
        let content = ecmascript_chunk_content(
            chunking_context,
            Vc::cell(vec![placeable]),
            availability_info,
        );
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
