use anyhow::Result;
use turbo_tasks::Value;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::issue::IssueContextExt;

use super::{
    ContentSource, ContentSourceContentVc, ContentSourceData, ContentSourceDataVaryVc,
    ContentSourceResult, ContentSourceResultVc, ContentSourceVc, ContentSourcesVc,
    GetContentSourceContent, GetContentSourceContentVc,
};

#[turbo_tasks::value]
pub struct IssueContextSource {
    context: Option<FileSystemPathVc>,
    description: String,
    source: ContentSourceVc,
}

#[turbo_tasks::value_impl]
impl IssueContextSourceVc {
    #[turbo_tasks::function]
    pub fn new_context(
        context: FileSystemPathVc,
        description: &str,
        source: ContentSourceVc,
    ) -> Self {
        IssueContextSource {
            context: Some(context),
            description: description.to_string(),
            source,
        }
        .cell()
    }

    #[turbo_tasks::function]
    pub fn new_description(description: &str, source: ContentSourceVc) -> Self {
        IssueContextSource {
            context: None,
            description: description.to_string(),
            source,
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl ContentSource for IssueContextSource {
    #[turbo_tasks::function]
    async fn get(
        self_vc: IssueContextSourceVc,
        path: &str,
        data: Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        let this = self_vc.await?;
        let result = this
            .source
            .get(path, data)
            .issue_context(this.context, &this.description)
            .await?;
        if let ContentSourceResult::Result {
            get_content,
            specificity,
        } = *result.await?
        {
            Ok(ContentSourceResult::Result {
                get_content: IssueContextGetContentSourceContent {
                    get_content,
                    source: self_vc,
                }
                .cell()
                .into(),
                specificity,
            }
            .cell())
        } else {
            Ok(result)
        }
    }

    #[turbo_tasks::function]
    fn get_children(&self) -> ContentSourcesVc {
        ContentSourcesVc::cell(vec![self.source])
    }
}

#[turbo_tasks::value]
struct IssueContextGetContentSourceContent {
    get_content: GetContentSourceContentVc,
    source: IssueContextSourceVc,
}

#[turbo_tasks::value_impl]
impl GetContentSourceContent for IssueContextGetContentSourceContent {
    #[turbo_tasks::function]
    async fn vary(&self) -> Result<ContentSourceDataVaryVc> {
        let source = self.source.await?;
        let result = self
            .get_content
            .vary()
            .issue_context(source.context, &source.description)
            .await?;
        Ok(result)
    }

    #[turbo_tasks::function]
    async fn get(&self, data: Value<ContentSourceData>) -> Result<ContentSourceContentVc> {
        let source = self.source.await?;
        let result = self
            .get_content
            .get(data)
            .issue_context(source.context, &source.description)
            .await?;
        Ok(result)
    }
}
