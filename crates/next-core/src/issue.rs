use anyhow::Result;
use turbo_tasks::primitives::StringVc;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::issue::{Issue, IssueSeverityVc, IssueVc};

#[turbo_tasks::value(shared)]
pub struct NextAppIssue {
    pub severity: IssueSeverityVc,
    pub path: FileSystemPathVc,
    pub message: StringVc,
}

#[turbo_tasks::value_impl]
impl Issue for NextAppIssue {
    #[turbo_tasks::function]
    fn severity(&self) -> IssueSeverityVc {
        self.severity
    }

    #[turbo_tasks::function]
    async fn title(&self) -> Result<StringVc> {
        Ok(StringVc::cell(
            "An issue occurred while preparing your Next.js app".to_string(),
        ))
    }

    #[turbo_tasks::function]
    fn category(&self) -> StringVc {
        StringVc::cell("next app".to_string())
    }

    #[turbo_tasks::function]
    fn context(&self) -> FileSystemPathVc {
        self.path
    }

    #[turbo_tasks::function]
    fn description(&self) -> StringVc {
        self.message
    }
}
