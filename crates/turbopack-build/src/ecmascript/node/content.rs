use std::{io::Write, sync::Arc};

use anyhow::{Context, Result};
use indexmap::IndexMap;
use indoc::writedoc;
use sourcemap::{self};
use swc_core::{
    base::{config::JsMinifyOptions, try_with_handler, BoolOrDataConfig, Compiler},
    common::{FileName, FilePathMapping, SourceMap as SwcSourceMap, GLOBALS},
};
use turbo_tasks::{TryJoinIterExt, Value, Vc};
use turbo_tasks_fs::File;
use turbopack_core::{
    asset::AssetContent,
    code_builder::{Code, CodeBuilder},
    output::OutputAsset,
    source_map::{
        GenerateSourceMap, OptionSourceMap, SourceMap as TurbopackSourceMap,
        Token as TurbopackSourceMapToken,
    },
};
use turbopack_ecmascript::{
    chunk::{EcmascriptChunkContent, EcmascriptChunkItemExt},
    utils::StringifyJs,
};

use super::chunk::EcmascriptBuildNodeChunk;
use crate::{chunking_context::MinifyType, BuildChunkingContext};

#[turbo_tasks::value]
pub(super) struct EcmascriptBuildNodeChunkContent {
    pub(super) content: Vc<EcmascriptChunkContent>,
    pub(super) chunking_context: Vc<BuildChunkingContext>,
    pub(super) chunk: Vc<EcmascriptBuildNodeChunk>,
}

#[turbo_tasks::value_impl]
impl EcmascriptBuildNodeChunkContent {
    #[turbo_tasks::function]
    pub(crate) async fn new(
        chunking_context: Vc<BuildChunkingContext>,
        chunk: Vc<EcmascriptBuildNodeChunk>,
        content: Vc<EcmascriptChunkContent>,
    ) -> Result<Vc<Self>> {
        Ok(EcmascriptBuildNodeChunkContent {
            content,
            chunking_context,
            chunk,
        }
        .cell())
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptBuildNodeChunkContent {
    #[turbo_tasks::function]
    async fn code(self: Vc<Self>) -> Result<Vc<Code>> {
        let this = self.await?;
        let chunk_path = this.chunk.ident().path().await?;

        let mut code = CodeBuilder::default();

        writedoc!(
            code,
            r#"
                module.exports = {{

            "#,
        )?;

        let content = this.content.await?;
        let availability_info = Value::new(content.availability_info);
        for (id, item_code) in content
            .chunk_items
            .iter()
            .map(|chunk_item| async move {
                Ok((
                    chunk_item.id().await?,
                    chunk_item.code(availability_info).await?,
                ))
            })
            .try_join()
            .await?
        {
            write!(code, "{}: ", StringifyJs(&id))?;
            code.push_code(&item_code);
            writeln!(code, ",")?;
        }

        write!(code, "\n}};")?;

        if code.has_source_map() {
            let filename = chunk_path.file_name();
            write!(code, "\n\n//# sourceMappingURL={}.map", filename)?;
        }

        let code = code.build();

        if matches!(
            this.chunking_context.await?.minify_type(),
            MinifyType::Minify
        ) {
            let mut builder = CodeBuilder::default();
            let cm = Arc::new(SwcSourceMap::new(FilePathMapping::empty()));
            let compiler = Arc::new(Compiler::new(cm));
            let fm = compiler.cm.new_source_file(
                FileName::Custom((*chunk_path.path).to_string()),
                code.source_code().to_str()?.to_string(),
            );

            let value = try_with_handler(compiler.cm.clone(), Default::default(), |handler| {
                GLOBALS.set(&Default::default(), || {
                    compiler
                        .minify(
                            fm,
                            handler,
                            &JsMinifyOptions {
                                source_map: BoolOrDataConfig::from_bool(true),
                                ..Default::default()
                            },
                        )
                        .context("failed to minify file")
                })
            })?;

            let original_map = *code.cell().generate_source_map().await?;
            let minify_map = match value.map {
                Some(m) => {
                    println!("swc minify map: {}", m);
                    Some(sourcemap::decode_slice(m.as_bytes())?)
                }
                None => None,
            };

            let merged = match (original_map, minify_map) {
                (Some(original_map), Some(minify_map)) => {
                    let minify_map = match minify_map {
                        sourcemap::DecodedMap::Regular(map) => map,
                        _ => panic!("Unexpected non-regular sourcemap"),
                    };

                    Some(Vc::upcast(trace_chunk_sourcemap(
                        original_map,
                        TurbopackSourceMap::new_regular(minify_map).cell(),
                    )))
                }
                _ => None,
            };

            builder.push_source(&value.code.into(), merged);
            return Ok(builder.build().cell());
        }

        Ok(code.cell())
    }

    #[turbo_tasks::function]
    pub async fn content(self: Vc<Self>) -> Result<Vc<AssetContent>> {
        let code = self.code().await?;
        Ok(AssetContent::file(
            File::from(code.source_code().clone()).into(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for EcmascriptBuildNodeChunkContent {
    #[turbo_tasks::function]
    fn generate_source_map(self: Vc<Self>) -> Vc<OptionSourceMap> {
        self.code().generate_source_map()
    }
}

#[turbo_tasks::function]
async fn trace_chunk_sourcemap(
    original: Vc<TurbopackSourceMap>,
    other: Vc<TurbopackSourceMap>,
) -> Result<Vc<TracedSourceMap>> {
    let original_map = match &*original.await? {
        TurbopackSourceMap::Regular(m) => m.clone(),
        TurbopackSourceMap::Sectioned(m) => m.flatten().await?,
    };

    let mut builder = sourcemap::SourceMapBuilder::new(original_map.get_file());

    let tokens = other
        .tokens()
        .await?
        .iter()
        .map(|other_token| async move {
            let other_token = match other_token {
                TurbopackSourceMapToken::Synthetic(_) => panic!("Unexpected synthetic token"),
                TurbopackSourceMapToken::Original(t) => t,
            };

            Ok((
                (*original
                    .lookup_token(other_token.original_line, other_token.original_column)
                    .await?)
                    .clone(),
                other_token.clone(),
            ))
        })
        .try_join()
        .await?;

    let mut source_to_src_id = IndexMap::new();
    for (original_token, other_token) in tokens {
        if let Some(original_token) = original_token {
            match original_token {
                TurbopackSourceMapToken::Original(original_token) => {
                    let token = builder.add(
                        other_token.generated_line as u32,
                        other_token.generated_column as u32,
                        original_token.original_line as u32,
                        original_token.original_column as u32,
                        Some(&original_token.original_file),
                        original_token.name.as_deref(),
                    );
                    source_to_src_id.insert(original_token.original_file, token.src_id);
                }
                TurbopackSourceMapToken::Synthetic(_original_token) => {
                    builder.add(
                        other_token.generated_line as u32,
                        other_token.generated_column as u32,
                        !0,
                        !0,
                        None,
                        None,
                    );
                }
            }
        }
    }

    for (src_id, source) in original_map.sources().enumerate() {
        builder.set_source_contents(
            *source_to_src_id.get(source).expect(&format!(
                "Expected source {} to exist in traced map",
                source
            )),
            original_map.get_source_contents(src_id as u32),
        );
    }

    Ok(
        TracedSourceMap::new(TurbopackSourceMap::new_regular(builder.into_sourcemap()).into())
            .cell(),
    )
}

#[turbo_tasks::value(shared)]
struct TracedSourceMap {
    map: Vc<TurbopackSourceMap>,
}

impl TracedSourceMap {
    fn new(map: Vc<TurbopackSourceMap>) -> Self {
        TracedSourceMap { map }
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for TracedSourceMap {
    #[turbo_tasks::function]
    fn generate_source_map(&self) -> Vc<OptionSourceMap> {
        Vc::cell(Some(self.map))
    }

    #[turbo_tasks::function]
    fn by_section(&self, _section: String) -> Vc<OptionSourceMap> {
        Vc::cell(None)
    }
}
