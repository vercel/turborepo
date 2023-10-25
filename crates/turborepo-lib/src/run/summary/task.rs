use std::collections::HashMap;

use itertools::Itertools;
use serde::Serialize;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_cache::CacheResponse;
use turborepo_env::{DetailedMap, EnvironmentVariableMap};

use super::execution::TaskExecutionSummary;
use crate::{cli::EnvMode, run::task_id::TaskId, task_graph::TaskDefinition};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskCacheSummary {
    // Deprecated, but keeping around for --dry=json
    local: bool,
    // Deprecated, but keeping around for --dry=json
    remote: bool,
    status: CacheStatus,
    // Present unless a cache miss
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<CacheSource>,
    // 0 if a cache miss
    time_saved: u64,
}

#[derive(Debug, Serialize, Copy, Clone)]
#[serde(rename_all = "UPPERCASE")]
enum CacheStatus {
    Hit,
    Miss,
}

#[derive(Debug, Serialize, Copy, Clone)]
#[serde(rename_all = "UPPERCASE")]
enum CacheSource {
    Local,
    Remote,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct TaskSummary {
    pub task_id: TaskId<'static>,
    pub dir: String,
    pub package: String,
    pub dependencies: Vec<TaskId<'static>>,
    pub dependents: Vec<TaskId<'static>>,
    #[serde(flatten)]
    pub shared: SharedTaskSummary,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct SingleTaskSummary {
    pub task_id: String,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
    #[serde(flatten)]
    pub shared: SharedTaskSummary,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct SharedTaskSummary {
    pub hash: String,
    pub expanded_inputs: HashMap<RelativeUnixPathBuf, String>,
    pub external_deps_hash: String,
    pub cache_summary: TaskCacheSummary,
    pub command: String,
    pub command_arguments: Vec<String>,
    pub outputs: Vec<String>,
    pub excluded_outputs: Vec<String>,
    pub log_file_relative_path: String,
    pub resolved_task_definition: TaskDefinition,
    pub expanded_outputs: Vec<AnchoredSystemPathBuf>,
    pub framework: String,
    // TODO: Do we really want this to be cli enum instead of the one defined in the parent module?
    pub env_mode: EnvMode,
    pub env_vars: TaskEnvVarSummary,
    pub dot_env: Vec<RelativeUnixPathBuf>,
    pub execution: Option<TaskExecutionSummary>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TaskEnvConfiguration {
    pub env: Vec<String>,
    // TODO: we most likely want this to be optional
    pub pass_through_env: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TaskEnvVarSummary {
    pub specified: TaskEnvConfiguration,

    pub configured: Vec<String>,
    pub inferred: Vec<String>,
    pub pass_through: Vec<String>,
}

impl TaskCacheSummary {
    pub fn cache_miss() -> Self {
        Self {
            local: false,
            remote: false,
            status: CacheStatus::Miss,
            time_saved: 0,
            source: None,
        }
    }
}

impl From<Option<CacheResponse>> for TaskCacheSummary {
    fn from(value: Option<CacheResponse>) -> Self {
        value.map_or_else(Self::cache_miss, |CacheResponse { source, time_saved }| {
            let source = CacheSource::from(source);
            // Assign these deprecated fields Local and Remote based on the information
            // available in the itemStatus. Note that these fields are
            // problematic, because an ItemStatus isn't always the composite
            // of both local and remote caches. That means that an ItemStatus might say it
            // was a local cache hit, and we return remote: false here. That's misleading
            // because it does not mean that there is no remote cache hit,
            // it _could_ mean that we never checked the remote cache. These
            // fields are being deprecated for this reason.
            let (local, remote) = match source {
                CacheSource::Local => (true, false),
                CacheSource::Remote => (false, true),
            };
            Self {
                local,
                remote,
                status: CacheStatus::Hit,
                source: Some(source),
                time_saved,
            }
        })
    }
}

impl From<turborepo_cache::CacheSource> for CacheSource {
    fn from(value: turborepo_cache::CacheSource) -> Self {
        match value {
            turborepo_cache::CacheSource::Local => Self::Local,
            turborepo_cache::CacheSource::Remote => Self::Remote,
        }
    }
}

impl TaskEnvVarSummary {
    pub fn new(
        task_definition: &TaskDefinition,
        env_vars: DetailedMap,
        env_at_execution_start: &EnvironmentVariableMap,
    ) -> Result<Self, regex::Error> {
        Ok(Self {
            specified: TaskEnvConfiguration {
                env: task_definition.env.clone(),
                pass_through_env: task_definition.pass_through_env.clone().unwrap_or_default(),
            },
            configured: env_vars.by_source.explicit.to_secret_hashable(),
            inferred: env_vars.by_source.matching.to_secret_hashable(),
            // TODO: this operation differs from the actual env that gets passed in during task
            // execution it should be unified, but first we should copy Go's behavior as
            // we try to match the implementations
            pass_through: env_at_execution_start
                .from_wildcards(
                    task_definition
                        .pass_through_env
                        .as_deref()
                        .unwrap_or_default(),
                )?
                .to_secret_hashable(),
        })
    }
}

impl From<TaskSummary> for SingleTaskSummary {
    fn from(value: TaskSummary) -> Self {
        let TaskSummary {
            task_id,
            dependencies,
            dependents,
            shared,
            ..
        } = value;
        Self {
            task_id: task_id.task().to_string(),
            dependencies: dependencies
                .into_iter()
                .map(|task_id| task_id.task().to_string())
                .sorted()
                .collect(),
            dependents: dependents
                .into_iter()
                .map(|task_id| task_id.task().to_string())
                .sorted()
                .collect(),
            shared,
        }
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    #[test_case(CacheStatus::Hit, json!("HIT") ; "hit")]
    #[test_case(CacheStatus::Miss, json!("MISS") ; "miss")]
    #[test_case(CacheSource::Local, json!("LOCAL") ; "local")]
    #[test_case(CacheSource::Remote, json!("REMOTE") ; "remote")]
    #[test_case(
        TaskCacheSummary::cache_miss(),
        serde_json::json!({
                "local": false,
                "remote": false,
                "status": "MISS",
                "timeSaved": 0,
            })
        ; "cache miss"
    )]
    #[test_case(
        TaskCacheSummary {
            local: true,
            remote: false,
            status: CacheStatus::Hit,
            source: Some(CacheSource::Local),
            time_saved: 6,
        },
        serde_json::json!({
                "local": true,
                "remote": false,
                "status": "HIT",
                "source": "LOCAL",
                "timeSaved": 6,
            })
        ; "local cache hit"
    )]
    fn test_serialization(value: impl serde::Serialize, expected: serde_json::Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
