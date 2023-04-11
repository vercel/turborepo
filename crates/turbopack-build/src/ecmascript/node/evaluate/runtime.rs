use std::io::Write;

use anyhow::{bail, Result};
use indoc::writedoc;
use turbo_tasks::{primitives::StringVc, Value, ValueToString, ValueToStringVc};
use turbo_tasks_fs::{embed_file, File, FileContent, FileSystem, FileSystemVc};
use turbopack_core::{
    asset::{Asset, AssetContentVc, AssetVc},
    chunk::ChunkingContext,
    code_builder::{CodeBuilder, CodeVc},
    ident::{AssetIdent, AssetIdentVc},
    reference::{AssetReference, AssetReferenceVc, AssetReferencesVc},
    resolve::{ResolveResult, ResolveResultVc},
    source_map::{
        GenerateSourceMap, GenerateSourceMapVc, OptionSourceMapVc, SourceMapAssetReferenceVc,
    },
    virtual_fs::VirtualFileSystemVc,
};
use turbopack_ecmascript::utils::StringifyJs;

use crate::BuildChunkingContextVc;

/// An Ecmascript chunk that contains the Node.js runtime code.
#[turbo_tasks::value(shared)]
pub(crate) struct EcmascriptBuildNodeRuntimeChunk {
    chunking_context: BuildChunkingContextVc,
}

#[turbo_tasks::value_impl]
impl EcmascriptBuildNodeRuntimeChunkVc {
    /// Creates a new [`EcmascriptBuildNodeRuntimeChunkVc`].
    #[turbo_tasks::function]
    pub fn new(chunking_context: BuildChunkingContextVc) -> Self {
        EcmascriptBuildNodeRuntimeChunk { chunking_context }.cell()
    }

    #[turbo_tasks::function]
    async fn code(self) -> Result<CodeVc> {
        let this = self.await?;

        let output_root = this.chunking_context.output_root().await?;
        let runtime_path = self.ident().path().await?;
        let runtime_public_path = if let Some(path) = output_root.get_path_to(&runtime_path) {
            path
        } else {
            bail!(
                "runtime path {} is not in output root {}",
                runtime_path.to_string(),
                output_root.to_string()
            );
        };

        let mut code = CodeBuilder::default();

        writedoc!(
            code,
            r#"
                const RUNTIME_PUBLIC_PATH = {};
            "#,
            StringifyJs(&*runtime_public_path)
        )?;

        let runtime_code = embed_file!("js/src/runtime.js").await?;

        match &*runtime_code {
            FileContent::NotFound => bail!("shared runtime code is not found"),
            FileContent::Content(file) => code.push_source(file.content(), None),
        };

        Ok(CodeVc::cell(code.build()))
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptBuildNodeRuntimeChunk {
    #[turbo_tasks::function]
    async fn to_string(&self) -> Result<StringVc> {
        Ok(StringVc::cell(
            "Ecmascript Build Node Runtime Chunk".to_string(),
        ))
    }
}

#[turbo_tasks::function]
fn runtime_fs() -> FileSystemVc {
    VirtualFileSystemVc::new().into()
}

#[turbo_tasks::value_impl]
impl Asset for EcmascriptBuildNodeRuntimeChunk {
    #[turbo_tasks::function]
    fn ident(&self) -> AssetIdentVc {
        let ident = AssetIdentVc::new(Value::new(AssetIdent {
            path: runtime_fs().root().join("runtime.js"),
            query: None,
            fragment: None,
            assets: Default::default(),
            modifiers: Default::default(),
            part: None,
        }));

        AssetIdentVc::from_path(self.chunking_context.chunk_path(ident, ".js"))
    }

    #[turbo_tasks::function]
    async fn references(self_vc: EcmascriptBuildNodeRuntimeChunkVc) -> Result<AssetReferencesVc> {
        let this = self_vc.await?;
        let mut references = vec![];

        if *this
            .chunking_context
            .reference_chunk_source_maps(self_vc.into())
            .await?
        {
            references.push(SourceMapAssetReferenceVc::new(self_vc.into()).into())
        }

        Ok(AssetReferencesVc::cell(references))
    }

    #[turbo_tasks::function]
    async fn content(self_vc: EcmascriptBuildNodeRuntimeChunkVc) -> Result<AssetContentVc> {
        let code = self_vc.code().await?;
        Ok(File::from(code.source_code().clone()).into())
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for EcmascriptBuildNodeRuntimeChunk {
    #[turbo_tasks::function]
    fn generate_source_map(self_vc: EcmascriptBuildNodeRuntimeChunkVc) -> OptionSourceMapVc {
        self_vc.code().generate_source_map()
    }
}

/// A reference to the runtime chunk.
#[turbo_tasks::value]
pub(crate) struct EcmascriptBuildNodeRuntimeReference {
    chunking_context: BuildChunkingContextVc,
}

#[turbo_tasks::value_impl]
impl EcmascriptBuildNodeRuntimeReferenceVc {
    #[turbo_tasks::function]
    pub fn new(chunking_context: BuildChunkingContextVc) -> Self {
        Self::cell(EcmascriptBuildNodeRuntimeReference { chunking_context })
    }

    #[turbo_tasks::function]
    pub async fn runtime_chunk(
        self_vc: EcmascriptBuildNodeRuntimeReferenceVc,
    ) -> Result<EcmascriptBuildNodeRuntimeChunkVc> {
        Ok(EcmascriptBuildNodeRuntimeChunkVc::new(
            self_vc.await?.chunking_context,
        ))
    }
}

#[turbo_tasks::value_impl]
impl AssetReference for EcmascriptBuildNodeRuntimeReference {
    #[turbo_tasks::function]
    fn resolve_reference(self_vc: EcmascriptBuildNodeRuntimeReferenceVc) -> ResolveResultVc {
        ResolveResult::asset(self_vc.runtime_chunk().into()).into()
    }
}

#[turbo_tasks::value_impl]
impl ValueToString for EcmascriptBuildNodeRuntimeReference {
    #[turbo_tasks::function]
    async fn to_string(self_vc: EcmascriptBuildNodeRuntimeReferenceVc) -> Result<StringVc> {
        Ok(StringVc::cell(format!(
            "runtime chunk {}",
            self_vc.runtime_chunk().ident().to_string().await?
        )))
    }
}
