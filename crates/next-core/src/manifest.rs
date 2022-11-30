use std::collections::HashSet;

use anyhow::Result;
use mime::APPLICATION_JSON;
use turbo_tasks::primitives::StringsVc;
use turbo_tasks_fs::File;
use turbopack_core::asset::AssetContentVc;
use turbopack_dev_server::source::{
    combined::CombinedContentSourceVc, conditional::ConditionalContentSourceVc, ContentSource,
    ContentSourceContent, ContentSourceData, ContentSourceResultVc, ContentSourceVc,
};

use crate::nodejs::{
    node_api_source::NodeApiContentSourceVc, node_rendered_source::NodeRenderContentSourceVc,
};

#[turbo_tasks::value(shared)]
pub struct DevManifestContentSource {
    pub page_roots: Vec<ContentSourceVc>,
}

#[turbo_tasks::value_impl]
impl DevManifestContentSourceVc {
    #[turbo_tasks::function]
    async fn find_routes(self) -> Result<StringsVc> {
        let this = &*self.await?;
        let mut queue = this.page_roots.clone();
        let mut routes = HashSet::new();

        while let Some(content_source) = queue.pop() {
            if let Some(combined_source) =
                CombinedContentSourceVc::resolve_from(content_source).await?
            {
                queue.extend(combined_source.await?.sources.iter().copied());

                continue;
            }

            if let Some(conditional_source) =
                ConditionalContentSourceVc::resolve_from(content_source).await?
            {
                let conditional_source = &*conditional_source.await?;
                queue.push(conditional_source.activator);
                queue.push(conditional_source.action);

                continue;
            }

            if let Some(api_source) = NodeApiContentSourceVc::resolve_from(content_source).await? {
                routes.insert(format!("/{}", api_source.await?.pathname.await?));

                continue;
            }

            if let Some(page_source) =
                NodeRenderContentSourceVc::resolve_from(content_source).await?
            {
                routes.insert(format!("/{}", page_source.await?.pathname.await?));

                continue;
            }

            // ignore anything else
        }

        Ok(StringsVc::cell(routes.into_iter().collect()))
    }
}

#[turbo_tasks::value_impl]
impl ContentSource for DevManifestContentSource {
    #[turbo_tasks::function]
    async fn get(
        self_vc: DevManifestContentSourceVc,
        path: &str,
        _data: turbo_tasks::Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        let requested_manifest = match path.rsplit_once('/') {
            Some(("_next/static/development", file))
                if file == "_devPagesManifest.json" || file == "_devMiddlewareManifest.json" =>
            {
                file
            }
            _ => return Ok(ContentSourceResultVc::not_found()),
        };

        let content = if requested_manifest == "_devPagesManifest.json" {
            let pages = &*self_vc.find_routes().await?;

            serde_json::to_string(&serde_json::json!({
                "pages": pages,
            }))?
        } else {
            // empty middleware manifest
            "[]".to_string()
        };
        let file = File::from(content).with_content_type(APPLICATION_JSON);

        Ok(ContentSourceResultVc::exact(
            ContentSourceContent::Static(AssetContentVc::from(file).into()).cell(),
        ))
    }
}
