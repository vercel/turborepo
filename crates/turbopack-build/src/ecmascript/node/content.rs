use std::io::Write;

use anyhow::{bail, Result};
use indoc::writedoc;
use turbo_tasks_fs::File;
use turbopack_core::{
    asset::{Asset, AssetContentVc},
    chunk::{ChunkingContext, ModuleId},
    code_builder::{CodeBuilder, CodeVc},
    source_map::{GenerateSourceMap, GenerateSourceMapVc, OptionSourceMapVc},
};
use turbopack_ecmascript::{chunk::EcmascriptChunkContentVc, utils::StringifyJs};

use super::{
    chunk::EcmascriptBuildNodeChunkVc, content_entry::EcmascriptBuildNodeChunkContentEntriesVc,
};
use crate::BuildChunkingContextVc;

#[turbo_tasks::value(serialization = "none")]
pub(super) struct EcmascriptBuildNodeChunkContent {
    pub(super) entries: EcmascriptBuildNodeChunkContentEntriesVc,
    pub(super) chunking_context: BuildChunkingContextVc,
    pub(super) chunk: EcmascriptBuildNodeChunkVc,
}

#[turbo_tasks::value_impl]
impl EcmascriptBuildNodeChunkContentVc {
    #[turbo_tasks::function]
    pub(crate) async fn new(
        chunking_context: BuildChunkingContextVc,
        chunk: EcmascriptBuildNodeChunkVc,
        content: EcmascriptChunkContentVc,
    ) -> Result<Self> {
        let entries = EcmascriptBuildNodeChunkContentEntriesVc::new(content)
            .resolve()
            .await?;
        Ok(EcmascriptBuildNodeChunkContent {
            entries,
            chunking_context,
            chunk,
        }
        .cell())
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptBuildNodeChunkContentVc {
    #[turbo_tasks::function]
    async fn code(self) -> Result<CodeVc> {
        let this = self.await?;
        let chunk_path = this.chunk.ident().path().await?;

        let mut code = CodeBuilder::default();

        // When a chunk is executed, it will either register itself with the current
        // instance of the runtime, or it will push itself onto the list of pending
        // chunks (`self.TURBOPACK`).
        //
        // When the runtime executes (see the `evaluate` module), it will pick up and
        // register all pending chunks, and replace the list of pending chunks
        // with itself so later chunks can register directly with it.
        writedoc!(
            code,
            r#"
                module.exports = {{
            "#,
        )?;

        for (id, entry) in this.entries.await?.iter() {
            write!(code, "\n{}: ", StringifyJs(&id))?;
            code.push_code(&*entry.code.await?);
            write!(code, ",")?;
        }

        write!(code, "\n}};")?;

        if code.has_source_map() {
            let filename = chunk_path.file_name();
            write!(code, "\n\n//# sourceMappingURL={}.map", filename)?;
        }

        let code = code.build();
        Ok(code.cell())
    }

    #[turbo_tasks::function]
    pub async fn content(self_vc: EcmascriptBuildNodeChunkContentVc) -> Result<AssetContentVc> {
        let code = self_vc.code().await?;
        Ok(File::from(code.source_code().clone()).into())
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for EcmascriptBuildNodeChunkContent {
    #[turbo_tasks::function]
    fn generate_source_map(self_vc: EcmascriptBuildNodeChunkContentVc) -> OptionSourceMapVc {
        self_vc.code().generate_source_map()
    }

    #[turbo_tasks::function]
    async fn by_section(&self, section: &str) -> Result<OptionSourceMapVc> {
        // Weirdly, the ContentSource will have already URL decoded the ModuleId, and we
        // can't reparse that via serde.
        if let Ok(id) = ModuleId::parse(section) {
            for (entry_id, entry) in self.entries.await?.iter() {
                if id == **entry_id {
                    let sm = entry.code.generate_source_map();
                    return Ok(sm);
                }
            }
        }

        Ok(OptionSourceMapVc::cell(None))
    }
}
