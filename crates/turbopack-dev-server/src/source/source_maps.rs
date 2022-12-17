use std::collections::HashSet;

use anyhow::{Context, Result};
use turbo_tasks::{primitives::StringVc, Value};
use turbo_tasks_fs::File;
use turbopack_core::{
    asset::AssetContentVc,
    introspect::{Introspectable, IntrospectableChildrenVc, IntrospectableVc},
    source_map::GenerateSourceMapVc,
};

use super::{
    ContentSource, ContentSourceContent, ContentSourceData, ContentSourceResultVc, ContentSourceVc,
};

/// SourceMapContentSource allows us to serve full source maps, and individual
/// sections of source maps, of any found asset in the graph without adding
/// the maps themselves to that graph.
///
/// Any path ending with `.map` is acceptable, and the stripped path will be
/// used to fetch from our wrapped ContentSource. Any found asset should
/// implement the [GenerateSourceMap] trait to generate full maps.
///
/// Optionally, if the path ends with `[{ID}].map`, we will instead fetch
/// an individual section from the asset via [GenerateSourceMap::by_section].
#[turbo_tasks::value(shared)]
pub struct SourceMapContentSource {
    /// A wrapped content source from which we will fetch assets.
    asset_source: ContentSourceVc,
}

#[turbo_tasks::value_impl]
impl SourceMapContentSourceVc {
    #[turbo_tasks::function]
    pub fn new(asset_source: ContentSourceVc) -> SourceMapContentSourceVc {
        SourceMapContentSource { asset_source }.cell()
    }
}

/// Extracts the contents between a `[` and `]` suffix.
fn extract_module_id(path: &str) -> (&str, Option<&str>) {
    if let Some(path) = path.strip_suffix(']') {
        if let Some((path, id)) = path.rsplit_once('[') {
            return (path, Some(id));
        }
    }

    (path, None)
}

#[turbo_tasks::value_impl]
impl ContentSource for SourceMapContentSource {
    #[turbo_tasks::function]
    async fn get(
        &self,
        path: &str,
        _data: Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        let path = path
            .strip_suffix(".map")
            .context("expected path to end with .map")?;
        let (pathname, id) = extract_module_id(path);

        let result = self
            .asset_source
            .get(pathname, Value::new(Default::default()))
            .await?;
        let file = match &*result.content.await? {
            ContentSourceContent::Static(f) => *f,
            _ => return Ok(ContentSourceResultVc::not_found()),
        };

        let gen = match GenerateSourceMapVc::resolve_from(file).await? {
            Some(f) => f,
            None => return Ok(ContentSourceResultVc::not_found()),
        };

        let sm = if let Some(id) = id {
            let section = gen.by_section(StringVc::cell(id.to_string())).await?;
            match &*section {
                Some(sm) => *sm,
                None => return Ok(ContentSourceResultVc::not_found()),
            }
        } else {
            gen.generate_source_map()
        };
        let content = sm.to_rope().await?;

        let asset = AssetContentVc::from(File::from(content));
        Ok(ContentSourceResultVc::exact(
            ContentSourceContent::Static(asset.into()).cell(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl Introspectable for SourceMapContentSource {
    #[turbo_tasks::function]
    fn ty(&self) -> StringVc {
        StringVc::cell("static assets directory content source".to_string())
    }

    #[turbo_tasks::function]
    fn children(&self) -> IntrospectableChildrenVc {
        IntrospectableChildrenVc::cell(HashSet::new())
    }
}
