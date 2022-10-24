use anyhow::Result;
use indexmap::IndexMap;

use crate::{EnvMapVc, OptionEnvValueVc, ProcessEnv, ProcessEnvVc};

/// Filters env variables by some prefix. Casing of the env vars is ignored for
/// filtering.
#[turbo_tasks::value]
pub struct FilterProcessEnv {
    prior: ProcessEnvVc,
    filters: Vec<String>,
}

#[turbo_tasks::value_impl]
impl FilterProcessEnvVc {
    #[turbo_tasks::function]
    pub fn new(prior: ProcessEnvVc, filters: Vec<String>) -> Self {
        FilterProcessEnv {
            prior,
            filters: filters.iter().map(|s| s.to_uppercase()).collect(),
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl ProcessEnv for FilterProcessEnv {
    #[turbo_tasks::function]
    async fn read_all(&self) -> Result<EnvMapVc> {
        let prior = self.prior.read_all().await?;
        let mut filtered = IndexMap::new();
        for (name, value) in &*prior {
            for filter in &self.filters {
                if name.to_uppercase().starts_with(filter) {
                    filtered.insert(name.clone(), value.clone());
                    break;
                }
            }
        }
        Ok(EnvMapVc::cell(filtered))
    }

    #[turbo_tasks::function]
    fn read(&self, name: &str) -> OptionEnvValueVc {
        for filter in &self.filters {
            if name.to_uppercase().starts_with(filter) {
                return self.prior.read(name);
            }
        }
        OptionEnvValueVc::cell(None)
    }
}
