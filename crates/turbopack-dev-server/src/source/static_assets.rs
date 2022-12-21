use std::collections::HashSet;

use anyhow::Result;
use turbo_tasks::{
    primitives::{OptionStringVc, StringVc},
    Value,
};
use turbo_tasks_fs::{DirectoryContent, DirectoryEntry, FileSystemEntryType, FileSystemPathVc};
use turbopack_core::{
    introspect::{
        asset::IntrospectableAssetVc, Introspectable, IntrospectableChildrenVc, IntrospectableVc,
    },
    source_asset::SourceAssetVc,
};

use super::{
    utils::strip_base_path, ContentSource, ContentSourceContent, ContentSourceData,
    ContentSourceResultVc, ContentSourceVc,
};

#[turbo_tasks::value(shared)]
pub struct StaticAssetsContentSource {
    pub base_path: OptionStringVc,
    pub dir: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl StaticAssetsContentSourceVc {
    #[turbo_tasks::function]
    pub fn new(base_path: OptionStringVc, dir: FileSystemPathVc) -> StaticAssetsContentSourceVc {
        StaticAssetsContentSource { base_path, dir }.cell()
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
            let base_path = self.base_path.await?;

            let path = if let Some(base_path) = base_path.as_deref() {
                strip_base_path(path, base_path)?
            } else {
                Some(path)
            };

            if let Some(path) = path {
                let path = self.dir.join(path);
                let ty = path.get_type().await?;
                if matches!(
                    &*ty,
                    FileSystemEntryType::File | FileSystemEntryType::Symlink
                ) {
                    let content = SourceAssetVc::new(path).as_asset().content();
                    return Ok(ContentSourceResultVc::exact(
                        ContentSourceContent::Static(content.into()).cell(),
                    ));
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
        let base_path_ref = self.base_path.await?;
        let base_path = base_path_ref.as_deref().unwrap_or("");
        let children = match &*dir {
            DirectoryContent::NotFound => HashSet::new(),
            DirectoryContent::Entries(entries) => entries
                .iter()
                .map(|(name, entry)| {
                    let child = match entry {
                        DirectoryEntry::File(path) | DirectoryEntry::Symlink(path) => {
                            IntrospectableAssetVc::new(SourceAssetVc::new(*path).as_asset())
                        }
                        DirectoryEntry::Directory(path) => StaticAssetsContentSourceVc::new(
                            OptionStringVc::cell(Some(format!("{base_path}/{name}",))),
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
