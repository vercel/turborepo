use anyhow::Result;
use lightningcss::{
    css_modules::{Pattern, Segment},
    stylesheet::{ParserOptions, StyleSheet},
};
use smallvec::smallvec;
use turbo_tasks::{ValueToString, Vc};
use turbo_tasks_fs::{FileContent, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    source::Source,
};

use crate::{CssModuleAssetType, ParseCssResult};

#[turbo_tasks::value(shared, serialization = "none")]
pub enum ProcessCssResult {
    Ok {
        #[turbo_tasks(trace_ignore)]
        output_code: String,
    },
    Unparseable,
    NotFound,
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

    Ok(ProcessCssResult::Ok {}.into())
}
