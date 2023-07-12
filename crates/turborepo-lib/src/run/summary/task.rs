use std::collections::HashMap;

use serde::Serialize;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{
    cli::EnvMode,
    run::{summary::execution::TaskExecutionSummary, task_id::TaskId},
    task_graph::TaskDefinition,
};

#[derive(Debug, Serialize)]
struct TaskCacheSummary {
    local: bool,
    remote: bool,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    time_saved: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskSummary<'a> {
    pub(crate) task_id: TaskId<'a>,
    package: Option<String>,
    hash: String,
    expanded_inputs: HashMap<RelativeUnixPathBuf, String>,
    external_deps_hash: String,
    cache_summary: TaskCacheSummary,
    command: String,
    command_arguments: Vec<String>,
    outputs: Vec<String>,
    excluded_outputs: Vec<String>,
    log_file_relative_path: String,
    dir: Option<String>,
    dependencies: Vec<TaskId<'a>>,
    dependents: Vec<TaskId<'a>>,
    resolved_task_definition: TaskDefinition,
    expanded_outputs: Vec<AnchoredSystemPathBuf>,
    framework: String,
    env_mode: EnvMode,
    env_vars: TaskEnvVarSummary,
    dot_env: Vec<RelativeUnixPathBuf>,
    execution: TaskExecutionSummary,
}

#[derive(Debug, Serialize)]
struct TaskEnvConfiguration {
    env: Vec<String>,
    pass_through_env: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TaskEnvVarSummary {
    specified: TaskEnvConfiguration,

    configured: Vec<String>,
    inferred: Vec<String>,
    pass_through: Vec<String>,
}

impl<'a> TaskSummary<'a> {
    pub fn clean_for_single_package(&mut self) {
        for dependency in &mut self.dependencies {
            dependency.strip_package();
        }

        for dependent in &mut self.dependents {
            dependent.strip_package()
        }

        self.task_id.strip_package();
        self.dir = None;
        self.package = None;
    }
}
