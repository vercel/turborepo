use std::env;

use anyhow::{anyhow, Context, Result};
use indexmap::IndexMap;
use turbo_tasks::ValueToString;
use turbo_tasks_fs::{FileContent, FileSystemPathVc};

use crate::{env_snapshot, EnvMapVc, EnvValue, ProcessEnv, ProcessEnvVc, GLOBAL_ENV_LOCK};

/// Load the environment variables defined via a dotenv file, with an
/// optional prior state that we can lookup already defined variables
/// from.
#[turbo_tasks::value]
pub struct DotenvProcessEnv {
    prior: Option<ProcessEnvVc>,
    path: FileSystemPathVc,
}

#[turbo_tasks::value_impl]
impl DotenvProcessEnvVc {
    #[turbo_tasks::function]
    pub fn new(prior: Option<ProcessEnvVc>, path: FileSystemPathVc) -> Self {
        DotenvProcessEnv { prior, path }.cell()
    }
}

#[turbo_tasks::value_impl]
impl ProcessEnv for DotenvProcessEnv {
    #[turbo_tasks::function]
    async fn read_all(&self) -> Result<EnvMapVc> {
        let prior = if let Some(p) = self.prior {
            Some(p.read_all().await?)
        } else {
            None
        };
        let empty = IndexMap::new();
        let prior = prior.as_deref().unwrap_or(&empty);

        let file = self.path.read().await?;
        if let FileContent::Content(f) = &*file {
            let res;
            let vars;
            {
                let lock = GLOBAL_ENV_LOCK.lock().unwrap();

                // Unfortunately, dotenvy only looks up variable references from the global env.
                // So we must mutate while we process. Afterwards, we can restore the initial
                // state.
                let initial = env_snapshot(&lock);

                restore_env(&initial, prior);

                // from_read will load parse and evalute the Read, and set variables
                // into the global env. If a later dotenv defines an already defined
                // var, it'll be ignored.
                res = dotenvy::from_read(f.read());

                vars = env_snapshot(&lock);
                restore_env(&vars, &initial);
            }

            if res.is_err() {
                res.context(anyhow!(
                    "unable to read {} for env vars",
                    self.path.to_string().await?
                ))?;
            }

            Ok(EnvMapVc::cell(vars))
        } else {
            Ok(EnvMapVc::cell(prior.clone()))
        }
    }
}

/// Restores the global env variables to mirror `to`.
fn restore_env(from: &IndexMap<String, EnvValue>, to: &IndexMap<String, EnvValue>) {
    for key in from.keys() {
        if !to.contains_key(key) {
            env::remove_var(key);
        }
    }
    for (key, value) in to {
        match from.get(key) {
            Some(v) if v == value => {}
            _ => env::set_var(key, value.to_str()),
        }
    }
}
