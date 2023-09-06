use std::collections::HashMap;

use anyhow::Result;
use lightningcss::{
    css_modules::{CssModuleExports, Pattern, Segment},
    dependencies::{Dependency, DependencyOptions},
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
};
use smallvec::smallvec;
use swc_core::base::sourcemap::SourceMapBuilder;
use turbo_tasks::{ValueToString, Vc};
use turbo_tasks_fs::{FileContent, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    chunk::ChunkingContext,
    reference::ModuleReferences,
    resolve::origin::ResolveOrigin,
    source::Source,
    source_map::{GenerateSourceMap, OptionSourceMap},
};

use crate::{
    lifetime_util::stylesheet_into_static,
    references::{
        analyze_references,
        url::{replace_url_references, resolve_url_reference},
    },
    CssModuleAssetType,
};

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub enum ProcessCssResult {
    Ok {
        #[turbo_tasks(trace_ignore)]
        stylesheet: StyleSheet<'static, 'static>,

        references: Vc<ModuleReferences>,
    },
    Unparseable,
    NotFound,
}

impl PartialEq for ProcessCssResult {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub enum FinalCssResult {
    Ok {
        #[turbo_tasks(trace_ignore)]
        output_code: String,

        #[turbo_tasks(trace_ignore)]
        exports: Option<CssModuleExports>,

        #[turbo_tasks(trace_ignore)]
        dependencies: Option<Vec<Dependency>>,

        source_map: Vc<ProcessCssResultSourceMap>,
    },
    Unparseable,
    NotFound,
}

impl PartialEq for FinalCssResult {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}

#[turbo_tasks::function]
pub async fn finalize_css(
    result: Vc<ProcessCssResult>,
    chunking_context: Vc<Box<dyn ChunkingContext>>,
) -> Result<Vc<FinalCssResult>> {
    let result = result.await?;
    match &*result {
        ProcessCssResult::Ok {
            stylesheet,
            references,
        } => {
            {
                let mut url_map = HashMap::new();

                for (src, reference) in url_references {
                    let resolved = resolve_url_reference(reference, chunking_context).await?;
                    if let Some(v) = resolved.as_ref().cloned() {
                        url_map.insert(src, v);
                    }
                }

                replace_url_references(&mut stylesheet, &url_map);
            }

            let mut srcmap = parcel_sourcemap::SourceMap::new("");
            let result = stylesheet.to_css(PrinterOptions {
                source_map: Some(&mut srcmap),
                analyze_dependencies: Some(DependencyOptions {
                    remove_imports: true,
                }),
                ..Default::default()
            })?;

            Ok(FinalCssResult::Ok {
                output_code: result.code,
                dependencies: result.dependencies,
                exports: result.exports,
                source_map: ProcessCssResultSourceMap::new(srcmap).cell(),
            }
            .into())
        }
        ProcessCssResult::Unparseable => Ok(FinalCssResult::Unparseable.into()),
        ProcessCssResult::NotFound => Ok(FinalCssResult::NotFound.into()),
    }
}

#[turbo_tasks::value_trait]
pub trait ProcessCss {
    async fn process_css(self: Vc<Self>) -> Result<Vc<ProcessCssResult>>;

    async fn finalize_css(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<FinalCssResult>>;
}

#[turbo_tasks::function]
pub async fn process_css(
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
    ty: CssModuleAssetType,
) -> Result<Vc<ProcessCssResult>> {
    let content = source.content();
    let fs_path = &*source.ident().path().await?;
    let ident_str = &*source.ident().to_string().await?;
    Ok(match &*content.await? {
        AssetContent::Redirect { .. } => ProcessCssResult::Unparseable.cell(),
        AssetContent::File(file) => match &*file.await? {
            FileContent::NotFound => ProcessCssResult::NotFound.cell(),
            FileContent::Content(file) => match file.content().to_str() {
                Err(_err) => ProcessCssResult::Unparseable.cell(),
                Ok(string) => {
                    process_content(string.into_owned(), fs_path, ident_str, source, origin, ty)
                        .await?
                }
            },
        },
    })
}

async fn process_content(
    code: String,
    fs_path: &FileSystemPath,
    ident_str: &str,
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
    ty: CssModuleAssetType,
) -> Result<Vc<ProcessCssResult>> {
    let config = ParserOptions {
        css_modules: match ty {
            CssModuleAssetType::Module => Some(lightningcss::css_modules::Config {
                pattern: Pattern {
                    segments: smallvec![
                        Segment::Local,
                        Segment::Literal("__"),
                        Segment::Name,
                        Segment::Literal("__"),
                        Segment::Hash,
                    ],
                },
                dashed_idents: false,
            }),

            _ => None,
        },
        filename: ident_str.to_string(),
        ..Default::default()
    };

    let stylesheet = match StyleSheet::parse(&code, config) {
        Ok(stylesheet) => stylesheet,
        Err(e) => {
            // TODO(kdy1): Report errors
            // e.to_diagnostics(&handler).emit();
            return Ok(ProcessCssResult::Unparseable.into());
        }
    };
    let mut stylesheet = stylesheet_into_static(stylesheet);

    let (references, url_references) = analyze_references(&mut stylesheet, source, origin)?;

    Ok(ProcessCssResult::Ok {
        stylesheet,
        references: Vc::cell(references),
    }
    .into())
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub struct ProcessCssResultSourceMap {
    #[turbo_tasks(debug_ignore, trace_ignore)]
    source_map: parcel_sourcemap::SourceMap,
}

impl PartialEq for ProcessCssResultSourceMap {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}

impl ProcessCssResultSourceMap {
    pub fn new(source_map: parcel_sourcemap::SourceMap) -> Self {
        ProcessCssResultSourceMap { source_map }
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for ProcessCssResultSourceMap {
    #[turbo_tasks::function]
    fn generate_source_map(&self) -> Vc<OptionSourceMap> {
        let mut builder = SourceMapBuilder::new(None);

        for src in self.source_map.get_sources() {
            builder.add_source(src);
        }

        for (idx, content) in self.source_map.get_sources_content().iter().enumerate() {
            builder.set_source_contents(idx as _, Some(&content));
        }

        for m in self.source_map.get_mappings() {
            builder.add(
                m.generated_line,
                m.generated_column,
                m.original.map(|v| v.original_line).unwrap_or_default(),
                m.original.map(|v| v.original_column).unwrap_or_default(),
                None,
                None,
            );
        }

        Vc::cell(Some(
            turbopack_core::source_map::SourceMap::new_regular(builder.into_sourcemap()).cell(),
        ))
    }
}
