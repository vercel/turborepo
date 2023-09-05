use anyhow::Result;
use lightningcss::{
    css_modules::{CssModuleExports, Pattern, Segment},
    dependencies::{Dependency, DependencyOptions},
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
};
use smallvec::smallvec;
use turbo_tasks::{ValueToString, Vc};
use turbo_tasks_fs::{FileContent, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    source::Source,
    source_map::{GenerateSourceMap, OptionSourceMap},
};

use crate::CssModuleAssetType;

#[turbo_tasks::value(shared, serialization = "none")]
pub enum ProcessCssResult {
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

#[turbo_tasks::value_trait]
pub trait ProcessCss {
    async fn process_css(self: Vc<Self>) -> Result<Vc<ProcessCssResult>>;
}

#[turbo_tasks::function]
pub async fn process_css(
    source: Vc<Box<dyn Source>>,
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
                    process_content(string.into_owned(), fs_path, ident_str, source, ty).await?
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

    let mut stylesheet = match StyleSheet::parse(&code, config) {
        Ok(stylesheet) => stylesheet,
        Err(e) => {
            // TODO(kdy1): Report errors
            // e.to_diagnostics(&handler).emit();
            return Ok(ProcessCssResult::Unparseable.into());
        }
    };

    let mut srcmap = parcel_sourcemap::SourceMap::new("");
    let result = stylesheet.to_css(PrinterOptions {
        source_map: Some(&mut srcmap),
        analyze_dependencies: Some(DependencyOptions {
            remove_imports: true,
        }),
        ..Default::default()
    })?;

    Ok(ProcessCssResult::Ok {
        output_code: result.code,
        dependencies: result.dependencies,
        exports: result.exports,
        source_map: ProcessCssResultSourceMap::new(srcmap).cell(),
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
        let map = self.source_map.build_source_map_with_config(
            &self.mappings,
            None,
            InlineSourcesContentConfig {},
        );
        Vc::cell(Some(
            turbopack_core::source_map::SourceMap::new_regular(map).cell(),
        ))
    }
}
