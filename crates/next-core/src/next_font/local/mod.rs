use anyhow::Result;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::resolve::{
    options::{
        ImportMapResult, ImportMapResultVc, ImportMapping, ImportMappingReplacement,
        ImportMappingReplacementVc, ImportMappingVc,
    },
    parse::{Request, RequestVc},
};

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
    async fn result(&self, request: RequestVc) -> Result<ImportMapResultVc> {
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

        Ok(ImportMapResult::NoEntry.into())
    }
}
