use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use turbopath::RelativeUnixPathBuf;

pub type Pipeline = HashMap<String, BookkeepingTaskDefinition>;

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct BookkeepingTaskDefinition {
    pub defined_fields: HashSet<String>,
    pub experimental_fields: HashSet<String>,
    pub experimental: TaskDefinitionExperiments,
    pub task_definition: TaskDefinitionHashable,
}

// A list of config fields in a task definition that are considered
// experimental. We keep these separated so we can compute a global hash without
// these.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDefinitionExperiments {
    pub(crate) pass_through_env: Vec<String>,
}

// TaskOutputs represents the patterns for including and excluding files from
// outputs
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskOutputs {
    pub inclusions: Vec<String>,
    pub exclusions: Vec<String>,
}

// TaskOutputMode defines the ways turbo can display task output during a run
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskOutputMode {
    // FullTaskOutput will show all task output
    #[default]
    Full,
    // None will hide all task output
    None,
    // Hash will display turbo-computed task hashes
    Hash,
    // New will show all new task output and turbo-computed task hashes for cached
    // output
    New,
    // Error will show task output for failures only; no cache miss/hit messages are
    // emitted
    Error,
}

// taskDefinitionHashable exists as a definition for PristinePipeline, which is
// used downstream for calculating the global hash. We want to exclude
// experimental fields here because we don't want experimental fields to be part
// of the global hash.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TaskDefinitionHashable {
    pub(crate) outputs: TaskOutputs,
    pub(crate) cache: bool,
    pub(crate) topological_dependencies: Vec<String>,
    pub(crate) task_dependencies: Vec<String>,
    pub(crate) inputs: Vec<String>,
    pub(crate) output_mode: TaskOutputMode,
    pub(crate) persistent: bool,
    pub(crate) env: Vec<String>,
    pub(crate) pass_through_env: Vec<String>,
    pub(crate) dot_env: Vec<RelativeUnixPathBuf>,
}

impl Default for TaskDefinitionHashable {
    fn default() -> Self {
        Self {
            outputs: TaskOutputs::default(),
            cache: true,
            topological_dependencies: Vec::new(),
            task_dependencies: Vec::new(),
            inputs: Vec::new(),
            output_mode: TaskOutputMode::default(),
            persistent: false,
            env: Vec::new(),
            pass_through_env: Vec::new(),
            dot_env: Vec::new(),
        }
    }
}

// Constructed from a RawTaskDefinition
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskDefinition {
    outputs: TaskOutputs,
    cache: bool,

    // This field is custom-marshalled from `env` and `depends_on``
    env_var_dependencies: Vec<String>,

    pass_through_env: Vec<String>,

    // TopologicalDependencies are tasks from package dependencies.
    // E.g. "build" is a topological dependency in:
    // dependsOn: ['^build'].
    // This field is custom-marshalled from rawTask.DependsOn
    topological_dependencies: Vec<String>,

    // TaskDependencies are anything that is not a topological dependency
    // E.g. both something and //whatever are TaskDependencies in:
    // dependsOn: ['something', '//whatever']
    // This field is custom-marshalled from rawTask.DependsOn
    task_dependencies: Vec<String>,

    // Inputs indicate the list of files this Task depends on. If any of those files change
    // we can conclude that any cached outputs or logs for this Task should be invalidated.
    inputs: Vec<String>,

    // OutputMode determines how we should log the output.
    output_mode: TaskOutputMode,

    // Persistent indicates whether the Task is expected to exit or not
    // Tasks marked Persistent do not exit (e.g. --watch mode or dev servers)
    persistent: bool,
}
