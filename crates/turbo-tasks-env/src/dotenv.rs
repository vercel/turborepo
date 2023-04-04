use std::{env, sync::MutexGuard};

use anyhow::{anyhow, Error, Result};
use indexmap::IndexMap;
use turbo_tasks::ValueToString;
use turbo_tasks_fs::{FileContent, FileSystemPathVc};

use crate::{EnvMapVc, ProcessEnv, ProcessEnvVc, GLOBAL_ENV_LOCK};

/// Load the environment variables defined via a dotenv file, with an
/// optional prior state that we can lookup already defined variables
/// from.
#[turbo_tasks::value]
pub struct DotenvProcessEnv {
    prior: Option<ProcessEnvVc>,
    path: FileSystemPathVc,
}

/// Dotenv loading depends on prior state to resolve the current state. This
/// exposes the origin of a failed parse, so that callers can determine if the
/// prior state failed, or parsing of the current dotenv failed.
pub enum DotenvReadResult {
    /// A PriorError is an error that happens during the read_all of the prior
    /// [ProcessEnvVc].
    PriorError(Error),

    /// A CurrentError is an error that happens during the read/parse of the
    /// `.env` file.
    CurrentError(Error),

    Ok(EnvMapVc),
}

impl DotenvProcessEnv {
    /// Attempts to assemble the EnvMapVc for our dotenv file. If either the
    /// prior fails to read, or the current dotenv can't be parsed, an
    /// appropriate Ok(DotenvReadResult) will be returned. If an unexpected
    /// error (like disk reading or remote cache access) fails, then a regular
    /// Err() will be returned.
    pub async fn try_read_all(&self) -> Result<DotenvReadResult> {
        let prior = match self.prior {
            None => None,
            Some(p) => match p.read_all().await {
                Ok(p) => Some(p),
                Err(e) => return Ok(DotenvReadResult::PriorError(e)),
            },
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
                let initial = env::vars().collect();

                restore_env(&initial, prior, &lock);

                // from_read will load parse and evalute the Read, and set variables
                // into the global env. If a later dotenv defines an already defined
                // var, it'll be ignored.
                res = dotenvy::from_read(f.read());

                vars = env::vars().collect();
                restore_env(&vars, &initial, &lock);
            }

            if let Err(e) = res {
                return Ok(DotenvReadResult::CurrentError(anyhow!(e).context(anyhow!(
                    "unable to read {} for env vars",
                    self.path.to_string().await?
                ))));
            }

            Ok(DotenvReadResult::Ok(EnvMapVc::cell(vars)))
        } else {
            Ok(DotenvReadResult::Ok(EnvMapVc::cell(prior.clone())))
        }
    }
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
    async fn read_all(self_vc: DotenvProcessEnvVc) -> Result<EnvMapVc> {
        let this = self_vc.await?;
        match this.try_read_all().await? {
            DotenvReadResult::Ok(v) => Ok(v),
            DotenvReadResult::PriorError(e) => Err(e),
            DotenvReadResult::CurrentError(e) => Err(e),
        }
    }
}

/// Restores the global env variables to mirror `to`.
fn restore_env(
    from: &IndexMap<String, String>,
    to: &IndexMap<String, String>,
    _lock: &MutexGuard<()>,
) {
    for key in from.keys() {
        if !to.contains_key(key) {
            env::remove_var(key);
        }
    }
    for (key, value) in to {
        match from.get(key) {
            Some(v) if v == value => {}
            _ => env::set_var(key, value),
        }
    }
}
