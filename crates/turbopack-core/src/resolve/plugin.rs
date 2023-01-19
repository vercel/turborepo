use anyhow::Result;
use turbo_tasks::primitives::BoolVc;
use turbo_tasks_fs::{glob::GlobVc, FileSystemPathVc};

use crate::resolve::{parse::RequestVc, ResolveResultVc};

#[turbo_tasks::value]
pub struct ResolvePluginCondition {
    root: FileSystemPathVc,
    glob: GlobVc,
}

#[turbo_tasks::value_impl]
impl ResolvePluginConditionVc {
    #[turbo_tasks::function]
    pub fn new(root: FileSystemPathVc, glob: GlobVc) -> Self {
        ResolvePluginCondition { root, glob }.cell()
    }

    #[turbo_tasks::function]
    pub async fn matches(self, fs_path: FileSystemPathVc) -> Result<BoolVc> {
        let this = self.await?;
        let root = this.root.await?;
        let glob = this.glob.await?;

        let path = fs_path.await?;

        if let Some(path) = root.get_path_to(&path) {
            if glob.execute(path) {
                return Ok(BoolVc::cell(true));
            }
        }

        Ok(BoolVc::cell(false))
    }
}

#[turbo_tasks::value(transparent)]
pub struct ResolveResultOption(Option<ResolveResultVc>);

#[turbo_tasks::value_impl]
impl ResolveResultOptionVc {
    #[turbo_tasks::function]
    pub fn some(result: ResolveResultVc) -> Self {
        ResolveResultOption(Some(result)).cell()
    }

    #[turbo_tasks::function]
    pub fn none() -> Self {
        ResolveResultOption(None).cell()
    }
}

#[turbo_tasks::value_trait]
pub trait ResolvePlugin {
    fn condition(&self) -> ResolvePluginConditionVc;
    fn after_resolve(&self, fs_path: FileSystemPathVc, request: RequestVc)
        -> ResolveResultOptionVc;
}
