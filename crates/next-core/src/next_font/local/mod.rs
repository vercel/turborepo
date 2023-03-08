use anyhow::{bail, Context, Result};
use turbo_tasks::Value;
use turbo_tasks_fs::{json::parse_json_with_source_context, FileSystemPathVc};
use turbopack_core::{
    resolve::{
        options::{
            ImportMapResult, ImportMapResultVc, ImportMapping, ImportMappingReplacement,
            ImportMappingReplacementVc, ImportMappingVc,
        },
        parse::{Request, RequestVc},
        pattern::QueryMapVc,
    },
    source_asset::SourceAssetVc,
};

use self::options::{options_from_request, NextFontLocalOptionsVc};

pub(crate) mod options;
pub(crate) mod request;

#[turbo_tasks::value(shared)]
pub(crate) struct NextFontLocalReplacer {
    project_path: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl NextFontLocalReplacerVc {
    #[turbo_tasks::function]
    pub fn new(project_path: FileSystemPathVc) -> Self {
        Self::cell(NextFontLocalReplacer { project_path })
    }
}

#[turbo_tasks::value_impl]
impl ImportMappingReplacement for NextFontLocalReplacer {
    #[turbo_tasks::function]
    fn replace(&self, _capture: &str) -> ImportMappingVc {
        ImportMapping::Ignore.into()
    }

    #[turbo_tasks::function]
    async fn result(
        &self,
        context: FileSystemPathVc,
        request: RequestVc,
    ) -> Result<ImportMapResultVc> {
        let request = &*request.await?;
        println!("RESULT CALLED");
        let Request::Module {
            module: _,
            path: _,
            query: query_vc
        } = request else {
            return Ok(ImportMapResult::NoEntry.into());
        };

        let query = query_vc.await?;
        println!("q {:?}", &*query);
        let options = font_options_from_query_map(*query_vc).await?;
        let mut assets: Vec<SourceAssetVc> = vec![];
        for font in &options.fonts {
            assets.push(SourceAssetVc::new(context.join(&font.path)));
        }
        Ok(ImportMapResult::NoEntry.into())
    }
}

#[turbo_tasks::function]
async fn font_options_from_query_map(query: QueryMapVc) -> Result<NextFontLocalOptionsVc> {
    let query_map = &*query.await?;
    // These are invariants from the next/font swc transform. Regular errors instead
    // of Issues should be okay.
    let query_map = query_map
        .as_ref()
        .context("next/font/local queries must exist")?;

    if query_map.len() != 1 {
        bail!("next/font/local queries must only have one entry");
    }

    let Some((json, _)) = query_map.iter().next() else {
            bail!("Expected one entry");
        };

    options_from_request(&parse_json_with_source_context(json)?)
        .map(|o| NextFontLocalOptionsVc::new(Value::new(o)))
}
