use anyhow::Result;

use crate::{EnvMapVc, ProcessEnv, ProcessEnvVc};

/// Allows providing any custom env values that you'd like, including the
/// ability to inject Literal values (which are injected into the resulting
/// module without string quoting).
#[turbo_tasks::value]
pub struct CustomProcessEnv {
    prior: ProcessEnvVc,
    custom: EnvMapVc,
}

#[turbo_tasks::value_impl]
impl CustomProcessEnvVc {
    #[turbo_tasks::function]
    pub fn new(prior: ProcessEnvVc, custom: EnvMapVc) -> Self {
        CustomProcessEnv { prior, custom }.cell()
    }
}

#[turbo_tasks::value_impl]
impl ProcessEnv for CustomProcessEnv {
    #[turbo_tasks::function]
    async fn read_all(&self) -> Result<EnvMapVc> {
        let prior = self.prior.read_all().await?;
        let custom = self.custom.await?;

        let mut extended = prior.clone_value();
        extended.extend(custom.clone_value());
        Ok(EnvMapVc::cell(extended))
    }
}
