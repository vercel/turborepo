use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    sync::{Arc, Mutex},
};

use serde::Deserialize;
use tracing::{debug, error, warn};
use turbopath::{AbsoluteSystemPathBuf, PathRelation};
use turborepo_cache::AsyncCache;
use turborepo_scm::SCM;
use turborepo_unescape::UnescapedString;

use super::ConfigCache;
use crate::{config::RawTurboJson, gitignore::ensure_turbo_is_gitignored};

// Environment variable key that will be used to enable, and set the expected
// trace location
const TASK_ACCESS_ENV_KEY: &str = "TURBOREPO_TRACE_FILE";
/// File name where the task is expected to leave a trace result
const TASK_ACCESS_TRACE_NAME: &str = "trace.json";
// Path to the config file that will be used to store the trace results
pub const TASK_ACCESS_CONFIG_PATH: [&str; 2] = [".turbo", "traced-config.json"];
/// File name where the task is expected to leave a trace result
const TURBO_CONFIG_FILE: &str = "turbo.json";

#[derive(Debug, thiserror::Error)]
pub enum ToFileError {
    #[error("Unable to serialize traced config: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("Unable to write traced config: {0}")]
    IO(#[from] std::io::Error),
    #[error("Unable to cache traced config: {0}")]
    Cache(#[from] turborepo_cache::CacheError),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskAccessTraceAccess {
    pub network: bool,
    pub file_paths: Vec<UnescapedString>,
    pub env_var_keys: Vec<UnescapedString>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskAccessTraceFile {
    pub accessed: TaskAccessTraceAccess,
    pub outputs: Vec<UnescapedString>,
}

#[derive(Deserialize, Debug)]
struct PackageJson {
    scripts: Option<std::collections::HashMap<String, String>>,
}

pub fn trace_file_path(
    repo_root: &AbsoluteSystemPathBuf,
    task_hash: &str,
) -> AbsoluteSystemPathBuf {
    repo_root.join_components(&[".turbo", task_hash, TASK_ACCESS_TRACE_NAME])
}

fn task_access_trace_enabled(repo_root: &AbsoluteSystemPathBuf) -> Result<bool, std::io::Error> {
    // TODO: use the existing config methods here
    let root_turbo_json_path = &repo_root.join_component(TURBO_CONFIG_FILE);
    if root_turbo_json_path.exists() {
        return Ok(false);
    }

    // read package.json at root
    let package_json_path = repo_root.join_components(&["package.json"]);
    let package_json_content = fs::read_to_string(package_json_path)?;
    let package: PackageJson = serde_json::from_str(&package_json_content)?;

    if let Some(scripts) = package.scripts {
        return match scripts.get("build") {
            Some(script) => Ok(script == "next build"),
            _ => Ok(false),
        };
    }

    Ok(false)
}

impl TaskAccessTraceFile {
    pub fn read(repo_root: &AbsoluteSystemPathBuf, task_hash: &str) -> Option<TaskAccessTraceFile> {
        let trace_file = trace_file_path(repo_root, task_hash);

        let Ok(f) = trace_file.open() else {
            return None;
        };

        match serde_json::from_reader(f) {
            Ok(trace) => Some(trace),
            Err(e) => {
                warn!("failed to parse trace file {trace_file}: {e}");
                None
            }
        }
    }

    pub fn can_cache(&self, repo_root: &AbsoluteSystemPathBuf) -> bool {
        // network
        if self.accessed.network {
            warn!(
                "skipping automatic task caching - detected network
        access",
            );
            return false;
        }

        // file system
        for unescaped_str in &self.accessed.file_paths {
            match AbsoluteSystemPathBuf::new(unescaped_str.to_string()) {
                Ok(path) => {
                    let relation = path.relation_to_path(repo_root);
                    // only paths within the repo can be automatically cached
                    if relation == PathRelation::Parent || relation == PathRelation::Divergent {
                        warn!(
                            "skipping automatic task caching - file accessed outside of repo root \
                             ({})",
                            unescaped_str
                        );
                        return false;
                    }
                }
                Err(e) => {
                    debug!("failed to parse path {unescaped_str}: {e}");
                }
            }
        }

        true
    }
}

#[derive(Clone)]
pub struct TaskAccess {
    pub repo_root: AbsoluteSystemPathBuf,
    trace_by_task: Arc<Mutex<HashMap<String, TaskAccessTraceFile>>>,
    config_cache: Option<ConfigCache>,
    enabled: bool,
}

impl TaskAccess {
    pub fn new(repo_root: AbsoluteSystemPathBuf, cache: AsyncCache, scm: &SCM) -> Self {
        let root = repo_root.clone();
        let enabled = task_access_trace_enabled(&root).unwrap_or(false);
        let trace_by_task = Arc::new(Mutex::new(HashMap::<String, TaskAccessTraceFile>::new()));
        let mut config_cache = Option::<ConfigCache>::None;

        // we only want to setup the config cacher if task access tracing is enabled
        if enabled {
            // make sure .turbo is ignored
            match ensure_turbo_is_gitignored(&repo_root) {
                Ok(_) => debug!("Automatically added .turbo to .gitignore"),
                Err(e) => {
                    error!("Failed to add .turbo to .gitignore. Caching will be disabled - {e}")
                }
            }

            let config_hash_result = ConfigCache::calculate_config_hash(scm, &root);
            if let Ok(c_hash) = config_hash_result {
                let c_cache = ConfigCache::new(
                    c_hash.to_string(),
                    root.clone(),
                    &TASK_ACCESS_CONFIG_PATH,
                    cache.clone(),
                );

                config_cache = Some(c_cache);
            }
        }

        Self {
            repo_root,
            trace_by_task,
            enabled,
            config_cache,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub async fn restore_config(&self) {
        match (self.enabled, &self.config_cache) {
            (true, Some(config_cache)) => match config_cache.restore().await {
                Ok(_) => debug!(
                    "TASK ACCESS TRACE: config restored for {}",
                    config_cache.hash()
                ),
                Err(_) => debug!(
                    "TASK ACCESS TRACE: no config found for {}",
                    config_cache.hash()
                ),
            },
            _ => {
                debug!("TASK ACCESS TRACE: unable to restore config from cache");
            }
        }
    }

    pub fn save_trace(&self, task_id: String, trace: TaskAccessTraceFile) {
        let trace_by_task = self.trace_by_task.lock();
        match trace_by_task {
            Ok(mut trace_by_task) => {
                trace_by_task.insert(task_id, trace);
            }
            Err(e) => {
                error!("Failed to save trace result - {e}");
            }
        }
    }

    pub fn get_env_var(&self, task_hash: &str) -> (String, AbsoluteSystemPathBuf) {
        let trace_file_path = trace_file_path(&self.repo_root, task_hash);
        (TASK_ACCESS_ENV_KEY.to_string(), trace_file_path)
    }

    pub async fn save(&self) {
        match self.to_file().await {
            Ok(_) => (),
            Err(e) => {
                error!("Failed to write task access trace file - {e}");
            }
        }
    }

    // Whether we can cache the given task, returning None if task access isn't
    // enabled or the trace can't be found
    pub fn can_cache(&self, task_hash: &str, task_id: &str) -> Option<bool> {
        if !self.is_enabled() {
            return None;
        }
        let trace = TaskAccessTraceFile::read(&self.repo_root, task_hash)?;
        if trace.can_cache(&self.repo_root) {
            self.save_trace(task_id.to_string(), trace);
            Some(true)
        } else {
            Some(false)
        }
    }

    async fn to_file(&self) -> Result<(), ToFileError> {
        // if task access tracing is not enabled, we don't need to do anything
        if !self.is_enabled() {
            return Ok(());
        }

        if let Some(config_cache) = &self.config_cache {
            let traced_config =
                RawTurboJson::from_task_access_trace(&self.trace_by_task.lock().unwrap());
            if traced_config.is_some() {
                // convert the traced_config to json and write the file to disk
                let traced_config_json = serde_json::to_string_pretty(&traced_config)?;
                let file_path = self.repo_root.join_components(&TASK_ACCESS_CONFIG_PATH);
                let mut file = File::create(file_path)?;
                write!(file, "{}", traced_config_json)?;
                file.flush()?;
                config_cache.save().await?;
            }
        }

        Ok(())
    }
}
