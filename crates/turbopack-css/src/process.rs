use std::{collections::HashMap, mem::transmute, sync::Arc};

use anyhow::Result;
use indexmap::IndexMap;
use lightningcss::{
    css_modules::{CssModuleExport, CssModuleExports, Pattern, Segment},
    dependencies::{Dependency, DependencyOptions},
    error::PrinterErrorKind,
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet, ToCssResult},
    targets::{Features, Targets},
    values::url::Url,
};
use smallvec::smallvec;
use swc_core::{
    atoms::Atom,
    base::sourcemap::SourceMapBuilder,
    common::{BytePos, FileName, LineCol},
    css::{
        codegen::{writer::basic::BasicCssWriter, CodeGenerator},
        modules::TransformConfig,
        visit::{VisitMut, VisitMutWith},
    },
};
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
use turbopack_swc_utils::emitter::IssueEmitter;

use crate::{
    lifetime_util::stylesheet_into_static,
    parse::InlineSourcesContentConfig,
    references::{
        analyze_references,
        url::{replace_url_references, resolve_url_reference, UrlAssetReference},
    },
    CssModuleAssetType,
};

#[derive(Debug)]
pub enum StyleSheetLike<'i, 'o> {
    LightningCss(StyleSheet<'i, 'o>),
    Swc {
        stylesheet: swc_core::css::ast::Stylesheet,
        css_modules: Option<SwcCssModuleMode>,
    },
}

#[derive(Debug)]
pub struct SwcCssModuleMode {
    basename: String,
    path_hash: u32,
}

impl PartialEq for StyleSheetLike<'_, '_> {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

pub type CssOutput = (ToCssResult, Option<ParseCssResultSourceMap>);

impl<'i, 'o> StyleSheetLike<'i, 'o> {
    pub fn to_static(
        &self,
        options: ParserOptions<'static, 'static>,
    ) -> StyleSheetLike<'static, 'static> {
        match self {
            StyleSheetLike::LightningCss(ss) => {
                StyleSheetLike::LightningCss(stylesheet_into_static(ss, options))
            }
            StyleSheetLike::Swc {
                stylesheet,
                css_modules,
            } => StyleSheetLike::Swc {
                stylesheet: stylesheet.clone(),
                css_modules: *css_modules,
            },
        }
    }

    pub fn to_css(
        &self,
        cm: Arc<swc_core::common::SourceMap>,
        enable_srcmap: bool,
        remove_imports: bool,
        handle_nesting: bool,
    ) -> Result<CssOutput, lightningcss::error::Error<PrinterErrorKind>> {
        match self {
            StyleSheetLike::LightningCss(ss) => {
                let mut srcmap = if enable_srcmap {
                    Some(parcel_sourcemap::SourceMap::new(""))
                } else {
                    None
                };

                let result = ss.to_css(PrinterOptions {
                    minify: true,
                    source_map: srcmap.as_mut(),
                    targets: if handle_nesting {
                        Targets {
                            include: Features::Nesting,
                            ..Default::default()
                        }
                    } else {
                        Default::default()
                    },
                    analyze_dependencies: Some(DependencyOptions { remove_imports }),
                    ..Default::default()
                })?;

                if let Some(srcmap) = &mut srcmap {
                    srcmap.add_sources(ss.sources.clone());
                }

                Ok((
                    result,
                    srcmap.map(ParseCssResultSourceMap::new_lightningcss),
                ))
            }
            StyleSheetLike::Swc {
                stylesheet,
                css_modules,
            } => {
                let mut stylesheet = stylesheet.clone();
                // We always analyze dependencies, but remove them only if remove_imports is
                // true
                let mut deps = vec![];
                stylesheet.visit_mut_with(&mut SwcDepColllector {
                    deps: &mut deps,
                    remove_imports,
                });

                // lightningcss specifies css module mode in the parser options.
                let mut css_module_exports = None;
                if let Some(SwcCssModuleMode {
                    basename,
                    path_hash,
                }) = css_modules
                {
                    let output = swc_core::css::modules::compile(
                        &mut stylesheet,
                        ModuleTransformConfig {
                            suffix: format!("__{}__{:x}", basename, path_hash),
                        },
                    );
                }

                if handle_nesting {
                    stylesheet.visit_mut_with(&mut swc_core::css::compat::compiler::Compiler::new(
                        swc_core::css::compat::compiler::Config {
                            process: swc_core::css::compat::feature::Features::NESTING,
                        },
                    ));
                }

                use swc_core::css::codegen::Emit;

                let mut code_string = String::new();
                let mut srcmap = if enable_srcmap { Some(vec![]) } else { None };

                let mut code_gen = CodeGenerator::new(
                    BasicCssWriter::new(&mut code_string, srcmap.as_mut(), Default::default()),
                    Default::default(),
                );

                code_gen.emit(&stylesheet)?;

                let srcmap =
                    srcmap.map(|srcmap| ParseCssResultSourceMap::new_swc(cm.clone(), srcmap));

                Ok((
                    ToCssResult {
                        code: code_string,
                        dependencies: Some(deps),
                        exports: css_module_exports,
                        references: None,
                    },
                    srcmap,
                ))
            }
        }
    }
}

/// Multiple [ModuleReference]s
#[turbo_tasks::value(transparent)]
pub struct UnresolvedUrlReferences(pub Vec<(String, Vc<UrlAssetReference>)>);

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub enum ParseCssResult {
    Ok {
        #[turbo_tasks(debug_ignore, trace_ignore)]
        cm: Arc<swc_core::common::SourceMap>,

        #[turbo_tasks(trace_ignore)]
        stylesheet: StyleSheetLike<'static, 'static>,

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
        #[turbo_tasks(debug_ignore, trace_ignore)]
        cm: Arc<swc_core::common::SourceMap>,

        #[turbo_tasks(trace_ignore)]
        stylesheet: StyleSheetLike<'static, 'static>,

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
            cm,
            stylesheet,
            references,
            url_references,
            options,
        } => {
            dbg!("process_css_with_placeholder => start");

            let stylesheet = stylesheet.to_static(options.clone());

            dbg!("process_css_with_placeholder => after stylesheet_into_static");

            let (result, _) = stylesheet.to_css(cm.clone(), false, false, false)?;

            dbg!("process_css_with_placeholder => after StyleSheet::to_css");

            let exports = result.exports.map(|exports| {
                let mut exports = exports.into_iter().collect::<IndexMap<_, _>>();

                exports.sort_keys();

                exports
            });

            dbg!("process_css_with_placeholder => after sorting exports");

            Ok(CssWithPlaceholderResult::Ok {
                cm: cm.clone(),
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
            cm,
            stylesheet,
            url_references,
            options,
            ..
        } => {
            dbg!("finalize_css => start");

            let mut stylesheet = stylesheet.to_static(options.clone());

            dbg!("finalize_css => after stylesheet_into_static");

            let url_references = *url_references;

            let mut url_map = HashMap::new();

            for (src, reference) in (*url_references.await?).iter() {
                let resolved = resolve_url_reference(*reference, chunking_context).await?;
                if let Some(v) = resolved.as_ref().cloned() {
                    url_map.insert(src.to_string(), v);
                }
            }

            replace_url_references(&mut stylesheet, &url_map);
            dbg!("finalize_css => after replacing url refs");

            let (result, srcmap) = stylesheet.to_css(cm.clone(), true, true, true)?;

            dbg!("finalize_css => after StyleSheet::to_css");

            Ok(FinalCssResult::Ok {
                output_code: result.code,
                exports: result.exports,
                source_map: srcmap.unwrap().cell(),
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
    use_lightningcss: bool,
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
                    process_content(
                        string.into_owned(),
                        fs_path,
                        ident_str,
                        source,
                        origin,
                        ty,
                        use_lightningcss,
                    )
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
    use_lightningcss: bool,
) -> Result<Vc<ParseCssResult>> {
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

    let cm: Arc<swc_core::common::SourceMap> = Default::default();

    let stylesheet = if use_lightningcss {
        StyleSheetLike::LightningCss(match StyleSheet::parse(&code, config.clone()) {
            Ok(stylesheet) => stylesheet_into_static(&stylesheet, clone_options(config.clone())),
            Err(_e) => {
                // TODO(kdy1): Report errors
                // e.to_diagnostics(&handler).emit();
                return Ok(ParseCssResult::Unparseable.into());
            }
        })
    } else {
        let handler = swc_core::common::errors::Handler::with_emitter(
            true,
            false,
            Box::new(IssueEmitter {
                source,
                source_map: cm.clone(),
                title: Some("Parsing css source code failed".to_string()),
            }),
        );

        let fm = cm.new_source_file(FileName::Custom(ident_str.to_string()), code);
        let mut errors = vec![];

        let ss = swc_core::css::parser::parse_file(
            &fm,
            Default::default(),
            swc_core::css::parser::parser::ParserConfig {
                css_modules: true,
                legacy_ie: true,
                ..Default::default()
            },
            &mut errors,
        );

        for err in errors {
            err.to_diagnostics(&handler).emit();
        }

        let ss = match ss {
            Ok(v) => v,
            Err(err) => {
                err.to_diagnostics(&handler).emit();
                return Ok(ParseCssResult::Unparseable.into());
            }
        };

        if handler.has_errors() {
            return Ok(ParseCssResult::Unparseable.into());
        }

        StyleSheetLike::Swc {
            stylesheet: ss,
            css_modules: matches!(ty, CssModuleAssetType::Module),
        }
    };

    let config = clone_options(config);
    let mut stylesheet = stylesheet.to_static(config.clone());

    let (references, url_references) = analyze_references(&mut stylesheet, source, origin)?;

    Ok(ParseCssResult::Ok {
        cm,
        stylesheet,
        references: Vc::cell(references),
        url_references: Vc::cell(url_references),
        options: config,
    }
    .into())
}

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub enum ParseCssResultSourceMap {
    Parcel {
        #[turbo_tasks(debug_ignore, trace_ignore)]
        source_map: parcel_sourcemap::SourceMap,
    },

    Swc {
        #[turbo_tasks(debug_ignore, trace_ignore)]
        source_map: Arc<swc_core::common::SourceMap>,

        /// The position mappings that can generate a real source map given a
        /// (SWC) SourceMap.
        #[turbo_tasks(debug_ignore, trace_ignore)]
        mappings: Vec<(BytePos, LineCol)>,
    },
}

impl PartialEq for ParseCssResultSourceMap {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl ParseCssResultSourceMap {
    pub fn new_lightningcss(source_map: parcel_sourcemap::SourceMap) -> Self {
        ParseCssResultSourceMap::Parcel { source_map }
    }

    pub fn new_swc(
        source_map: Arc<swc_core::common::SourceMap>,
        mappings: Vec<(BytePos, LineCol)>,
    ) -> Self {
        ParseCssResultSourceMap::Swc {
            source_map,
            mappings,
        }
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for ParseCssResultSourceMap {
    #[turbo_tasks::function]
    fn generate_source_map(&self) -> Vc<OptionSourceMap> {
        match self {
            ParseCssResultSourceMap::Parcel { source_map } => {
                let mut builder = SourceMapBuilder::new(None);

                for src in source_map.get_sources() {
                    builder.add_source(src);
                }

                for (idx, content) in source_map.get_sources_content().iter().enumerate() {
                    builder.set_source_contents(idx as _, Some(content));
                }

                for m in source_map.get_mappings() {
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
                    turbopack_core::source_map::SourceMap::new_regular(builder.into_sourcemap())
                        .cell(),
                ))
            }
            ParseCssResultSourceMap::Swc {
                source_map,
                mappings,
            } => {
                let map = source_map.build_source_map_with_config(
                    mappings,
                    None,
                    InlineSourcesContentConfig {},
                );
                Vc::cell(Some(
                    turbopack_core::source_map::SourceMap::new_regular(map).cell(),
                ))
            }
        }
    }
}

struct SwcDepColllector<'a> {
    deps: &'a mut Vec<Dependency>,
    remove_imports: bool,
}

impl VisitMut for SwcDepColllector<'_> {}

struct ModuleTransformConfig {
    suffix: String,
}

impl TransformConfig for ModuleTransformConfig {
    fn new_name_for(&self, local: &Atom) -> Atom {
        format!("{}{}", *local, self.suffix).into()
    }
}
