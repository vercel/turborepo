use anyhow::Result;
use turbo_tasks::primitives::StringVc;
use turbo_tasks_env::{
    dotenv::DotenvReadResult, DotenvProcessEnvVc, EnvMapVc, ProcessEnv, ProcessEnvVc,
};
use turbo_tasks_fs::FileSystemPathVc;

use crate::ProcessEnvIssue;

#[turbo_tasks::value]
pub struct TryDotenvProcessEnv {
    dotenv: DotenvProcessEnvVc,
    prior: ProcessEnvVc,
    path: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl TryDotenvProcessEnvVc {
    #[turbo_tasks::function]
    pub fn new(prior: ProcessEnvVc, path: FileSystemPathVc) -> Self {
        let dotenv = DotenvProcessEnvVc::new(Some(prior), path);
        TryDotenvProcessEnv {
            dotenv,
            prior,
            path,
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl ProcessEnv for TryDotenvProcessEnv {
    #[turbo_tasks::function]
    async fn read_all(&self) -> Result<EnvMapVc> {
        let dotenv = self.dotenv.await?;
        match dotenv.try_read_all().await? {
            DotenvReadResult::Ok(v) => Ok(v),
            DotenvReadResult::PriorError(e) => {
                // If reading the prior value failed, then we cannot determine what the read
                // could have been (the dotenv depends on the state of the prior to build)
                // build). Trust the prior will emit an issue, and error out.
                Err(e)
            }
            DotenvReadResult::CurrentError(e) => {
                // If parsing the dotenv file fails (but getting the prior value didn't), then
                // we want to emit an error and fall back to the prior's read.
                ProcessEnvIssue {
                    path: self.path,
                    // try_read_all will wrap a current error with a context containing the failing
                    // file, which we don't really care about (we report the filepath as the Issue
                    // context, not the description). So extract the real error.
                    description: StringVc::cell(e.root_cause().to_string()),
                }
                .cell()
                .as_issue()
                .emit();
                Ok(self.prior.read_all())
            }
        }
    }
}
