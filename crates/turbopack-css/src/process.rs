use std::{collections::HashMap, mem::transmute};

use anyhow::Result;
use indexmap::IndexMap;
use lightningcss::{
    css_modules::{CssModuleExport, CssModuleExports, Pattern, Segment},
    dependencies::{Dependency, DependencyOptions},
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
    targets::{Features, Targets},
    values::url::Url,
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
        url::{replace_url_references, resolve_url_reference, UrlAssetReference},
    },
    CssModuleAssetType,
};

/// Multiple [ModuleReference]s
#[turbo_tasks::value(transparent)]
pub struct UnresolvedUrlReferences(pub Vec<(String, Vc<UrlAssetReference>)>);

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub enum ParseCssResult {
    Ok {
        #[turbo_tasks(trace_ignore)]
        stylesheet: StyleSheet<'static, 'static>,

        references: Vc<ModuleReferences>,

        url_references: Vc<UnresolvedUrlReferences>,

        #[turbo_tasks(trace_ignore)]
        options: ParserOptions<'static, 'static>,
    },
    Unparseable,
    NotFound,
}

impl PartialEq for ParseCssResult {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub enum CssWithPlaceholderResult {
    Ok {
        #[turbo_tasks(trace_ignore)]
        stylesheet: StyleSheet<'static, 'static>,

        references: Vc<ModuleReferences>,

        url_references: Vc<UnresolvedUrlReferences>,

        #[turbo_tasks(trace_ignore)]
        exports: Option<IndexMap<String, CssModuleExport>>,

        #[turbo_tasks(trace_ignore)]
        placeholders: HashMap<String, Url<'static>>,

        #[turbo_tasks(trace_ignore)]
        options: ParserOptions<'static, 'static>,
    },
    Unparseable,
    NotFound,
}

impl PartialEq for CssWithPlaceholderResult {
    fn eq(&self, _: &Self) -> bool {
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

        source_map: Vc<ParseCssResultSourceMap>,
    },
    Unparseable,
    NotFound,
}

impl PartialEq for FinalCssResult {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

#[turbo_tasks::function]
pub async fn process_css_with_placeholder(
    result: Vc<ParseCssResult>,
) -> Result<Vc<CssWithPlaceholderResult>> {
    let result = result.await?;

    match &*result {
        ParseCssResult::Ok {
            stylesheet,
            references,
            url_references,
            options,
        } => {
            let stylesheet = stylesheet_into_static(stylesheet, options.clone());

            let result = stylesheet.to_css(PrinterOptions {
                analyze_dependencies: Some(DependencyOptions {
                    ..Default::default()
                }),
                ..Default::default()
            })?;

            let exports = result.exports.map(|exports| {
                let mut exports = exports.into_iter().collect::<IndexMap<_, _>>();

                exports.sort_keys();

                exports
            });

            Ok(CssWithPlaceholderResult::Ok {
                exports,
                references: *references,
                url_references: *url_references,
                placeholders: HashMap::new(),
                stylesheet,
                options: options.clone(),
            }
            .into())
        }
        ParseCssResult::Unparseable => Ok(CssWithPlaceholderResult::Unparseable.into()),
        ParseCssResult::NotFound => Ok(CssWithPlaceholderResult::NotFound.into()),
    }
}

#[turbo_tasks::function]
pub async fn finalize_css(
    result: Vc<CssWithPlaceholderResult>,
    chunking_context: Vc<Box<dyn ChunkingContext>>,
) -> Result<Vc<FinalCssResult>> {
    let result = result.await?;
    match &*result {
        CssWithPlaceholderResult::Ok {
            stylesheet,
            url_references,
            options,
            ..
        } => {
            let mut stylesheet = stylesheet_into_static(stylesheet, options.clone());

            let url_references = *url_references;

            let mut url_map = HashMap::new();

            for (src, reference) in (*url_references.await?).iter() {
                let resolved = resolve_url_reference(*reference, chunking_context).await?;
                if let Some(v) = resolved.as_ref().cloned() {
                    url_map.insert(src.to_string(), v);
                }
            }

            replace_url_references(&mut stylesheet, &url_map);

            let mut srcmap = parcel_sourcemap::SourceMap::new("");
            let result = stylesheet.to_css(PrinterOptions {
                source_map: Some(&mut srcmap),
                analyze_dependencies: Some(DependencyOptions {
                    remove_imports: true,
                }),
                targets: Targets {
                    include: Features::Nesting,
                    ..Default::default()
                },
                ..Default::default()
            })?;

            srcmap.add_sources(stylesheet.sources.clone());

            Ok(FinalCssResult::Ok {
                output_code: result.code,
                exports: result.exports,
                source_map: ParseCssResultSourceMap::new(srcmap).cell(),
            }
            .into())
        }
        CssWithPlaceholderResult::Unparseable => Ok(FinalCssResult::Unparseable.into()),
        CssWithPlaceholderResult::NotFound => Ok(FinalCssResult::NotFound.into()),
    }
}

#[turbo_tasks::value_trait]
pub trait ParseCss {
    async fn parse_css(self: Vc<Self>) -> Result<Vc<ParseCssResult>>;
}

#[turbo_tasks::value_trait]
pub trait ProcessCss: ParseCss {
    async fn get_css_with_placeholder(self: Vc<Self>) -> Result<Vc<CssWithPlaceholderResult>>;

    async fn finalize_css(
        self: Vc<Self>,
        chunking_context: Vc<Box<dyn ChunkingContext>>,
    ) -> Result<Vc<FinalCssResult>>;
}

#[turbo_tasks::function]
pub async fn parse_css(
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
    ty: CssModuleAssetType,
) -> Result<Vc<ParseCssResult>> {
    let content = source.content();
    let fs_path = &*source.ident().path().await?;
    let ident_str = &*source.ident().to_string().await?;
    Ok(match &*content.await? {
        AssetContent::Redirect { .. } => ParseCssResult::Unparseable.cell(),
        AssetContent::File(file) => match &*file.await? {
            FileContent::NotFound => ParseCssResult::NotFound.cell(),
            FileContent::Content(file) => match file.content().to_str() {
                Err(_err) => ParseCssResult::Unparseable.cell(),
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
    _fs_path: &FileSystemPath,
    ident_str: &str,
    source: Vc<Box<dyn Source>>,
    origin: Vc<Box<dyn ResolveOrigin>>,
    ty: CssModuleAssetType,
) -> Result<Vc<ParseCssResult>> {
    let config = ParserOptions {
        css_modules: match ty {
            CssModuleAssetType::Module => Some(lightningcss::css_modules::Config {
                pattern: Pattern {
                    segments: smallvec![
                        Segment::Name,
                        Segment::Literal("__"),
                        Segment::Hash,
                        Segment::Literal("__"),
                        Segment::Local,
                    ],
                },
                dashed_idents: false,
            }),

            _ => None,
        },
        filename: ident_str.to_string(),
        ..Default::default()
    };

    let stylesheet = match StyleSheet::parse(&code, config.clone()) {
        Ok(stylesheet) => stylesheet,
        Err(_e) => {
            // TODO(kdy1): Report errors
            // e.to_diagnostics(&handler).emit();
            return Ok(ParseCssResult::Unparseable.into());
        }
    };

    fn clone_options(config: ParserOptions) -> ParserOptions<'static, 'static> {
        ParserOptions {
            filename: config.filename,
            css_modules: config
                .css_modules
                .clone()
                .map(|v| lightningcss::css_modules::Config {
                    pattern: Pattern {
                        segments: unsafe {
                            // Safety: It's actually static (two `__``)
                            transmute(v.pattern.segments.clone())
                        },
                    },
                    dashed_idents: v.dashed_idents,
                }),
            source_index: config.source_index,
            error_recovery: config.error_recovery,
            warnings: None,
            flags: config.flags,
        }
    }

    let config = clone_options(config);
    let mut stylesheet = stylesheet_into_static(&stylesheet, config.clone());

    let (references, url_references) = analyze_references(&mut stylesheet, source, origin)?;

    Ok(ParseCssResult::Ok {
        stylesheet,
        references: Vc::cell(references),
        url_references: Vc::cell(url_references),
        options: config,
    }
    .into())
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub struct ParseCssResultSourceMap {
    #[turbo_tasks(debug_ignore, trace_ignore)]
    source_map: parcel_sourcemap::SourceMap,
}

impl PartialEq for ParseCssResultSourceMap {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl ParseCssResultSourceMap {
    pub fn new(source_map: parcel_sourcemap::SourceMap) -> Self {
        ParseCssResultSourceMap { source_map }
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for ParseCssResultSourceMap {
    #[turbo_tasks::function]
    fn generate_source_map(&self) -> Vc<OptionSourceMap> {
        let mut builder = SourceMapBuilder::new(None);

        for src in self.source_map.get_sources() {
            builder.add_source(src);
        }

        for (idx, content) in self.source_map.get_sources_content().iter().enumerate() {
            builder.set_source_contents(idx as _, Some(content));
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
