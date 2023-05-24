use anyhow::Result;
use turbo_tasks::Value;
use turbo_tasks_fs::{FileContent, FileSystemPathVc};
use turbopack_core::asset::AssetContentVc;

use super::{ContentSource, ContentSourceData, ContentSourceResultVc, ContentSourceVc};
use crate::source::ContentSourceContentVc;

#[turbo_tasks::value(shared)]
pub struct FilesContentSource {
    root: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl FilesContentSourceVc {
    #[turbo_tasks::function]
    pub fn new(root: FileSystemPathVc) -> Self {
        FilesContentSource { root }.cell()
    }
}

#[turbo_tasks::value_impl]
impl ContentSource for FilesContentSource {
    #[turbo_tasks::function]
    async fn get(
        self_vc: FilesContentSourceVc,
        path: &str,
        _data: Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        let this = self_vc.await?;
        let path = this.root.join(path);
        Ok(match &*path.read().await? {
            FileContent::Content(f) => {
                let asset = AssetContentVc::from(f.clone());
                let static_content = ContentSourceContentVc::static_content(asset.into());
                ContentSourceResultVc::exact(static_content.into())
            }
            FileContent::NotFound => ContentSourceResultVc::not_found(),
        })
    }
}
