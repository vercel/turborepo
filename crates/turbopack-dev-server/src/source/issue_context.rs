use anyhow::Result;
use turbo_tasks::{CollectiblesSource, Value};
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::issue::IssueVc;

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

impl IssueContextSource {
    async fn attach<T: CollectiblesSource + Copy>(&self, source: T) -> Result<T> {
        IssueVc::attach_context_or_description(self.context, &self.description, source).await
    }
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
        let result = this.source.get(path, data);
        let result = this.attach(result).await?;
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
        let result = self.get_content.vary();
        let result = self.source.await?.attach(result).await?;
        Ok(result)
    }

    #[turbo_tasks::function]
    async fn get(&self, data: Value<ContentSourceData>) -> Result<ContentSourceContentVc> {
        let result = self.get_content.get(data);
        let result = self.source.await?.attach(result).await?;
        Ok(result)
    }
}
