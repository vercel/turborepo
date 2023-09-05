use std::sync::Arc;

use anyhow::Result;
use indexmap::IndexMap;
use lightningcss::{
    css_modules::{Pattern, Segment},
    stylesheet::{ParserOptions, StyleSheet},
};
use once_cell::sync::Lazy;
use regex::Regex;
use smallvec::smallvec;
use swc_core::{
    common::{
        errors::Handler, source_map::SourceMapGenConfig, BytePos, FileName, LineCol, SourceMap,
    },
    css::{
        ast::Stylesheet,
        modules::{CssClassName, TransformConfig},
        parser::{parse_file, parser::ParserConfig},
    },
    ecma::atoms::JsWord,
};
use turbo_tasks::{ValueToString, Vc};
use turbo_tasks_fs::{FileContent, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    source::Source,
    source_map::{GenerateSourceMap, OptionSourceMap},
    SOURCE_MAP_ROOT_NAME,
};
use turbopack_swc_utils::emitter::IssueEmitter;

use crate::{
    transform::{CssInputTransform, CssInputTransforms, TransformContext},
    CssModuleAssetType,
};

// Capture up until the first "."
static BASENAME_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[^.]*").unwrap());

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
pub struct ParseCssResultSourceMap {
    #[turbo_tasks(debug_ignore, trace_ignore)]
    source_map: Arc<SourceMap>,

    /// The position mappings that can generate a real source map given a (SWC)
    /// SourceMap.
    #[turbo_tasks(debug_ignore, trace_ignore)]
    mappings: parcel_sourcemap::SourceMap,
}

impl PartialEq for ParseCssResultSourceMap {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source_map, &other.source_map) && self.mappings == other.mappings
    }
}

impl ParseCssResultSourceMap {
    pub fn new(source_map: Arc<SourceMap>, mappings: parcel_sourcemap::SourceMap) -> Self {
        ParseCssResultSourceMap {
            source_map,
            mappings,
        }
    }
}

#[turbo_tasks::value_impl]
impl GenerateSourceMap for ParseCssResultSourceMap {
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

/// A config to generate a source map which includes the source content of every
/// source file. SWC doesn't inline sources content by default when generating a
/// sourcemap, so we need to provide a custom config to do it.
struct InlineSourcesContentConfig {}

impl SourceMapGenConfig for InlineSourcesContentConfig {
    fn file_name_to_source(&self, f: &FileName) -> String {
        match f {
            FileName::Custom(s) => format!("/{SOURCE_MAP_ROOT_NAME}/{s}"),
            _ => f.to_string(),
        }
    }

    fn inline_sources_content(&self, _f: &FileName) -> bool {
        true
    }
}
