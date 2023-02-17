use anyhow::Result;
use turbo_tasks::{primitives::StringVc, Value};
use turbo_tasks_fs::{DirectoryContent, DirectoryEntry, FileSystemEntryType, FileSystemPathVc};
use turbopack_core::{
    asset::Asset,
    introspect::{
        asset::IntrospectableAssetVc, Introspectable, IntrospectableChildrenVc, IntrospectableVc,
    },
    source_asset::SourceAssetVc,
};

use super::{
    ContentSource, ContentSourceContentVc, ContentSourceData, ContentSourceResultVc,
    ContentSourceVc,
};

#[turbo_tasks::value(shared)]
pub struct StaticAssetsContentSource {
    pub prefix: StringVc,
    pub dir: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl StaticAssetsContentSourceVc {
    #[turbo_tasks::function]
    pub fn new(prefix: StringVc, dir: FileSystemPathVc) -> StaticAssetsContentSourceVc {
        StaticAssetsContentSource { prefix, dir }.cell()
    }
}

#[turbo_tasks::value_impl]
impl ContentSource for StaticAssetsContentSource {
    #[turbo_tasks::function]
    async fn get(
        &self,
        path: &str,
        _data: Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        if !path.is_empty() {
            let prefix = self.prefix.await?;
            // If the prefix isn't empty, we need to strip the leading '/'.
            let prefix = prefix.strip_prefix('/').unwrap_or(&prefix);
            if let Some(path) = path.strip_prefix(prefix) {
                if prefix.is_empty() || path.starts_with('/') {
                    let path = self.dir.join(path);
                    let ty = path.get_type().await?;
                    if matches!(
                        &*ty,
                        FileSystemEntryType::File | FileSystemEntryType::Symlink
                    ) {
                        let content = SourceAssetVc::new(path).as_asset().content();
                        return Ok(ContentSourceResultVc::exact(
                            ContentSourceContentVc::static_content(content.into()).into(),
                        ));
                    }
                }
            }
        }
        Ok(ContentSourceResultVc::not_found())
    }
}

#[turbo_tasks::value_impl]
impl Introspectable for StaticAssetsContentSource {
    #[turbo_tasks::function]
    fn ty(&self) -> StringVc {
        StringVc::cell("static assets directory content source".to_string())
    }

    #[turbo_tasks::function]
    async fn children(&self) -> Result<IntrospectableChildrenVc> {
        let dir = self.dir.read_dir().await?;
        let prefix = self.prefix.await?;
        let children = match &*dir {
            DirectoryContent::NotFound => Default::default(),
            DirectoryContent::Entries(entries) => entries
                .iter()
                .map(|(name, entry)| {
                    let child = match entry {
                        DirectoryEntry::File(path) | DirectoryEntry::Symlink(path) => {
                            IntrospectableAssetVc::new(SourceAssetVc::new(*path).as_asset())
                        }
                        DirectoryEntry::Directory(path) => StaticAssetsContentSourceVc::new(
                            StringVc::cell(format!("{prefix}/{name}")),
                            *path,
                        )
                        .into(),
                        DirectoryEntry::Other(_) => todo!("what's DirectoryContent::Other?"),
                        DirectoryEntry::Error => todo!(),
                    };
                    (StringVc::cell(name.clone()), child)
                })
                .collect(),
        };
        Ok(IntrospectableChildrenVc::cell(children))
    }
}
