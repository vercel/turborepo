use anyhow::{bail, Result};
use turbo_tasks::{primitives::StringVc, Value, ValueToString, ValueToStringVc};
use turbopack_core::{
    asset::Asset,
    chunk::{ChunkGroupVc, ChunkListReferenceVc, ChunkingContext},
    ident::AssetIdentVc,
    reference::AssetReferencesVc,
};
use turbopack_ecmascript::chunk::{
    EcmascriptChunkContextVc, EcmascriptChunkRuntime, EcmascriptChunkRuntimeContentVc,
    EcmascriptChunkRuntimeVc, EcmascriptChunkVc,
};

use crate::ecmascript::content::EcmascriptDevChunkContentVc;

/// Development runtime for Ecmascript chunks.
#[turbo_tasks::value(shared)]
pub(crate) struct EcmascriptDevChunkRuntime {
    /// The chunking context that created this runtime.
    chunking_context: EcmascriptChunkContextVc,
    /// All chunks of this chunk group need to be ready for execution to start.
    /// When None, it will use a chunk group created from the current chunk.
    chunk_group: Option<ChunkGroupVc>,
    /// The mode of this runtime.
    mode: EcmascriptDevChunkRuntimeMode,
}

#[turbo_tasks::value_impl]
impl EcmascriptDevChunkRuntimeVc {
    /// Creates a new [`EcmascriptDevChunkRuntimeVc`].
    #[turbo_tasks::function]
    pub fn new(
        chunking_context: EcmascriptChunkContextVc,
        mode: Value<EcmascriptDevChunkRuntimeMode>,
    ) -> Self {
        EcmascriptDevChunkRuntime {
            chunking_context,
            chunk_group: None,
            mode: mode.into_value(),
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptDevChunkRuntime {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell("Ecmascript Dev Runtime".to_string()))
    }
}

#[turbo_tasks::function]
fn modifier() -> StringVc {
    StringVc::cell("ecmascript dev chunk".to_string())
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkRuntime for EcmascriptDevChunkRuntime {
    #[turbo_tasks::function]
    async fn decorate_asset_ident(
        &self,
        origin_chunk: EcmascriptChunkVc,
        ident: AssetIdentVc,
    ) -> Result<AssetIdentVc> {
        let Self {
            chunking_context: _,
            chunk_group,
            mode,
        } = self;

        let mut ident = ident.await?.clone_value();

        // Add a constant modifier to qualify this runtime.
        ident.add_modifier(modifier());

        // Only add other modifiers when the chunk is evaluated. Otherwise, it will
        // not receive any params and as such won't differ from another chunk in a
        // different chunk group.
        if matches!(mode, EcmascriptDevChunkRuntimeMode::RegisterAndEvaluate) {
            ident.modifiers.extend(
                origin_chunk
                    .main_entries()
                    .await?
                    .iter()
                    .map(|entry| entry.ident().to_string()),
            );

            // When the chunk group has changed, e.g. due to optimization, we want to
            // include the information too. Since the optimization is
            // deterministic, it's enough to include the entry chunk which is the only
            // factor that influences the chunk group chunks.
            // We want to avoid a cycle when this chunk is the entry chunk.
            if let Some(chunk_group) = chunk_group {
                let entry = chunk_group.entry().resolve().await?;
                if entry != origin_chunk.into() {
                    ident.add_modifier(entry.ident().to_string());
                }
            }
        }

        Ok(AssetIdentVc::new(Value::new(ident)))
    }

    #[turbo_tasks::function]
    fn with_chunk_group(&self, chunk_group: ChunkGroupVc) -> EcmascriptDevChunkRuntimeVc {
        EcmascriptDevChunkRuntimeVc::cell(EcmascriptDevChunkRuntime {
            chunking_context: self.chunking_context,
            chunk_group: Some(chunk_group),
            mode: self.mode,
        })
    }

    #[turbo_tasks::function]
    fn references(&self, origin_chunk: EcmascriptChunkVc) -> AssetReferencesVc {
        let Self {
            chunk_group,
            chunking_context,
            mode: _,
        } = self;

        let chunk_group =
            chunk_group.unwrap_or_else(|| ChunkGroupVc::from_chunk(origin_chunk.into()));
        AssetReferencesVc::cell(vec![ChunkListReferenceVc::new(
            chunking_context.output_root(),
            chunk_group,
        )
        .into()])
    }

    #[turbo_tasks::function]
    fn content(&self, origin_chunk: EcmascriptChunkVc) -> EcmascriptChunkRuntimeContentVc {
        EcmascriptDevChunkContentVc::new(
            origin_chunk,
            self.chunking_context,
            self.chunk_group,
            Value::new(self.mode),
        )
        .into()
    }

    #[turbo_tasks::function]
    async fn merge(
        &self,
        runtimes: Vec<EcmascriptChunkRuntimeVc>,
    ) -> Result<EcmascriptChunkRuntimeVc> {
        let Self {
            chunking_context,
            chunk_group,
            mut mode,
        } = self;

        let chunking_context = chunking_context.resolve().await?;
        let chunk_group = if let Some(chunk_group) = chunk_group {
            Some(chunk_group.resolve().await?)
        } else {
            None
        };

        for runtime in runtimes {
            let Some(runtime) = EcmascriptDevChunkRuntimeVc::resolve_from(runtime).await? else {
                bail!("cannot merge EcmascriptDevChunkRuntime with non-EcmascriptDevChunkRuntime");
            };

            let Self {
                chunking_context: other_chunking_context,
                chunk_group: other_chunk_group,
                mode: other_mode,
            } = &*runtime.await?;

            let other_chunking_context = other_chunking_context.resolve().await?;
            let other_chunk_group = if let Some(other_chunk_group) = other_chunk_group {
                Some(other_chunk_group.resolve().await?)
            } else {
                None
            };

            if chunking_context != other_chunking_context {
                bail!("cannot merge EcmascriptDevChunkRuntime with different chunking contexts",);
            }

            if chunk_group != other_chunk_group {
                bail!("cannot merge EcmascriptDevChunkRuntime with different chunk groups",);
            }

            mode |= *other_mode;
        }

        Ok(EcmascriptDevChunkRuntime {
            chunking_context,
            chunk_group,
            mode,
        }
        .cell()
        .into())
    }

    #[turbo_tasks::function]
    fn evaluated(&self) -> EcmascriptChunkRuntimeVc {
        EcmascriptDevChunkRuntime {
            chunking_context: self.chunking_context,
            chunk_group: self.chunk_group,
            mode: EcmascriptDevChunkRuntimeMode::RegisterAndEvaluate,
        }
        .cell()
        .into()
    }
}

#[turbo_tasks::value(serialization = "auto_for_input")]
#[derive(Debug, Clone, Copy, PartialOrd, Ord, Hash)]
pub(crate) enum EcmascriptDevChunkRuntimeMode {
    /// The main runtime code will be included in the chunk and the main entries
    /// will be evaluated as soon as the chunk executes.
    RegisterAndEvaluate,
    /// The chunk's entries will be registered within an existing runtime.
    Register,
}

impl std::ops::BitOr for EcmascriptDevChunkRuntimeMode {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::RegisterAndEvaluate, _) => Self::RegisterAndEvaluate,
            (_, Self::RegisterAndEvaluate) => Self::RegisterAndEvaluate,
            _ => Self::Register,
        }
    }
}

impl std::ops::BitOrAssign for EcmascriptDevChunkRuntimeMode {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}
