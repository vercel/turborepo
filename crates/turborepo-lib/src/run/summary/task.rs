use std::collections::HashMap;

use serde::Serialize;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{
    cli::EnvMode,
    run::{summary::execution::TaskExecutionSummary, task_id::TaskId},
    task_graph::TaskDefinition,
};

#[derive(Debug, Serialize)]
pub struct TaskCacheSummary {
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
    pub package: Option<String>,
    pub hash: String,
    pub expanded_inputs: HashMap<RelativeUnixPathBuf, String>,
    pub external_deps_hash: String,
    pub cache_summary: TaskCacheSummary,
    pub command: String,
    pub command_arguments: Vec<String>,
    pub outputs: Vec<String>,
    pub excluded_outputs: Vec<String>,
    pub log_file_relative_path: String,
    pub dir: Option<String>,
    pub dependencies: Vec<TaskId<'a>>,
    pub dependents: Vec<TaskId<'a>>,
    pub resolved_task_definition: TaskDefinition,
    pub expanded_outputs: Vec<AnchoredSystemPathBuf>,
    pub framework: String,
    pub env_mode: EnvMode,
    pub env_vars: TaskEnvVarSummary,
    pub dot_env: Vec<RelativeUnixPathBuf>,
    pub execution: TaskExecutionSummary,
}

#[derive(Debug, Serialize)]
pub struct TaskEnvConfiguration {
    pub env: Vec<String>,
    pub pass_through_env: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskEnvVarSummary {
    pub specified: TaskEnvConfiguration,

    pub configured: Vec<String>,
    pub inferred: Vec<String>,
    pub pass_through: Vec<String>,
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
