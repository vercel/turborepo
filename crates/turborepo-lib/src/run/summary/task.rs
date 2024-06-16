use std::collections::BTreeMap;

use serde::Serialize;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_cache::CacheHitMetadata;
use turborepo_env::{DetailedMap, EnvironmentVariableMap};

use super::{execution::TaskExecutionSummary, EnvMode};
use crate::{
    cli::OutputLogsMode,
    run::task_id::TaskId,
    task_graph::{TaskDefinition, TaskOutputs},
};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskCacheSummary {
    // Deprecated, but keeping around for --dry=json
    pub local: bool,
    // Deprecated, but keeping around for --dry=json
    pub remote: bool,
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
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskSummary {
    pub task_id: TaskId<'static>,
    pub task: String,
    pub package: String,
    #[serde(flatten)]
    pub shared: SharedTaskSummary<TaskId<'static>>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SinglePackageTaskSummary {
    pub task_id: String,
    pub task: String,
    #[serde(flatten)]
    pub shared: SharedTaskSummary<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SharedTaskSummary<T> {
    pub hash: String,
    pub inputs: BTreeMap<RelativeUnixPathBuf, String>,
    pub hash_of_external_dependencies: String,
    pub cache: TaskCacheSummary,
    pub command: String,
    pub cli_arguments: Vec<String>,
    pub outputs: Option<Vec<String>>,
    pub excluded_outputs: Option<Vec<String>>,
    pub log_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    pub dependencies: Vec<T>,
    pub dependents: Vec<T>,
    pub resolved_task_definition: TaskSummaryTaskDefinition,
    pub expanded_outputs: Vec<AnchoredSystemPathBuf>,
    pub framework: String,
    pub env_mode: EnvMode,
    pub environment_variables: TaskEnvVarSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<TaskExecutionSummary>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskEnvConfiguration {
    pub env: Vec<String>,
    pub pass_through_env: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummaryTaskDefinition {
    outputs: Vec<String>,
    cache: bool,
    depends_on: Vec<String>,
    inputs: Vec<String>,
    output_logs: OutputLogsMode,
    persistent: bool,
    env: Vec<String>,
    pass_through_env: Option<Vec<String>>,
    interactive: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskEnvVarSummary {
    pub specified: TaskEnvConfiguration,
    pub configured: Vec<String>,
    pub inferred: Vec<String>,
    #[serde(rename = "passthrough")]
    pub pass_through: Option<Vec<String>>,
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

impl From<Option<CacheHitMetadata>> for TaskCacheSummary {
    fn from(response: Option<CacheHitMetadata>) -> Self {
        match response {
            Some(CacheHitMetadata { source, time_saved }) => {
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
            }
            None => Self::cache_miss(),
        }
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
    ) -> Result<Self, turborepo_env::Error> {
        // TODO: this operation differs from the actual env that gets passed in during
        // task execution it should be unified, but first we should copy Go's
        // behavior as we try to match the implementations
        let pass_through = task_definition
            .pass_through_env
            .as_deref()
            .map(|pass_through_env| -> Result<_, turborepo_env::Error> {
                Ok(env_at_execution_start
                    .from_wildcards(pass_through_env)?
                    .to_secret_hashable())
            })
            .transpose()?;

        Ok(Self {
            specified: TaskEnvConfiguration {
                env: task_definition.env.clone(),
                pass_through_env: task_definition.pass_through_env.clone(),
            },
            configured: env_vars.by_source.explicit.to_secret_hashable(),
            inferred: env_vars.by_source.matching.to_secret_hashable(),
            pass_through,
        })
    }
}

impl From<TaskSummary> for SinglePackageTaskSummary {
    fn from(value: TaskSummary) -> Self {
        let TaskSummary {
            task_id, shared, ..
        } = value;
        Self {
            task_id: task_id.task().to_string(),
            task: task_id.task().to_string(),
            shared: shared.into(),
        }
    }
}

impl From<SharedTaskSummary<TaskId<'static>>> for SharedTaskSummary<String> {
    fn from(value: SharedTaskSummary<TaskId<'static>>) -> Self {
        let SharedTaskSummary {
            hash,
            inputs,
            hash_of_external_dependencies,
            cache,
            command,
            cli_arguments,
            outputs,
            excluded_outputs,
            log_file,
            expanded_outputs,
            dependencies,
            dependents,
            resolved_task_definition,
            framework,
            execution,
            env_mode,
            environment_variables,
            ..
        } = value;
        Self {
            hash,
            inputs,
            hash_of_external_dependencies,
            cache,
            command,
            cli_arguments,
            outputs,
            excluded_outputs,
            log_file,
            directory: None,
            expanded_outputs,
            dependencies: dependencies
                .into_iter()
                .map(|task_id| task_id.task().to_string())
                .collect(),
            dependents: dependents
                .into_iter()
                .map(|task_id| task_id.task().to_string())
                .collect(),
            resolved_task_definition,
            framework,
            execution,
            env_mode,
            environment_variables,
        }
    }
}

impl From<TaskDefinition> for TaskSummaryTaskDefinition {
    fn from(value: TaskDefinition) -> Self {
        let TaskDefinition {
            outputs:
                TaskOutputs {
                    inclusions,
                    exclusions,
                },
            cache,
            mut env,
            pass_through_env,
            topological_dependencies,
            task_dependencies,
            mut inputs,
            output_logs,
            persistent,
            interactive,
        } = value;

        let mut outputs = inclusions;
        for exclusion in exclusions {
            outputs.push(format!("!{exclusion}"));
        }

        let mut depends_on =
            Vec::with_capacity(task_dependencies.len() + topological_dependencies.len());
        for task_dependency in task_dependencies {
            depends_on.push(task_dependency.to_string());
        }
        for topological_dependency in topological_dependencies {
            depends_on.push(format!("^{}", topological_dependency.as_inner()));
        }

        // These _should_ already be sorted when the TaskDefinition struct was
        // unmarshaled, but we want to ensure they're sorted on the way out
        // also, just in case something in the middle mutates the items.
        depends_on.sort();
        outputs.sort();
        env.sort();
        inputs.sort();

        Self {
            outputs,
            cache,
            depends_on,
            inputs,
            output_logs,
            persistent,
            interactive,
            env,
            pass_through_env,
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
    #[test_case(
        TaskSummaryTaskDefinition {
            outputs: vec!["foo".into()],
            cache: true,
            ..Default::default()
        },
        json!({
            "outputs": ["foo"],
            "cache": true,
            "dependsOn": [],
            "inputs": [],
            "outputLogs": "full",
            "persistent": false,
            "interactive": false,
            "env": [],
            "passThroughEnv": null,
        })
        ; "resolved task definition"
    )]
    fn test_serialization(value: impl serde::Serialize, expected: serde_json::Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
