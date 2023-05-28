use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

pub type Pipeline = HashMap<String, BookkeepingTaskDefinition>;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BookkeepingTaskDefinition {
    defined_fields: HashSet<String>,
    experimental_fields: HashSet<String>,
    experimental: TaskDefinitionExperiments,
    task_definition: TaskDefinitionHashable,
}

// A list of config fields in a task definition that are considered
// experimental. We keep these separated so we can compute a global hash without
// these.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskDefinitionExperiments {
    passthrough_env: Vec<String>,
}

// TaskOutputs represents the patterns for including and excluding files from
// outputs
#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskOutputs {
    inclusions: Vec<String>,
    exclusions: Vec<String>,
}

// TaskOutputMode defines the ways turbo can display task output during a run
#[derive(Debug, Default, Serialize, Deserialize)]
enum TaskOutputMode {
    // FullTaskOutput will show all task output
    #[default]
    FullTaskOutput,
    // NoTaskOutput will hide all task output
    NoTaskOutput,
    // HashTaskOutput will display turbo-computed task hashes
    HashTaskOutput,
    // NewTaskOutput will show all new task output and turbo-computed task hashes for cached
    // output
    NewTaskOutput,
    // ErrorTaskOutput will show task output for failures only; no cache miss/hit messages are
    // emitted
    ErrorTaskOutput,
}

// taskDefinitionHashable exists as a definition for PristinePipeline, which is
// used downstream for calculating the global hash. We want to exclude
// experimental fields here because we don't want experimental fields to be part
// of the global hash.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskDefinitionHashable {
    outputs: TaskOutputs,
    should_cache: bool,
    env_var_dependencies: Vec<String>,
    topological_dependencies: Vec<String>,
    task_dependencies: Vec<String>,
    inputs: Vec<String>,
    output_mode: TaskOutputMode,
    persistent: bool,
}

// task_definition is a representation of the configFile pipeline for further
// computation.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TaskDefinition {
    outputs: TaskOutputs,
    should_cache: bool,

    // This field is custom-marshalled from rawTask.Env and rawTask.DependsOn
    env_var_dependencies: Vec<String>,

    // rawTask.PassthroughEnv
    passthrough_env: Vec<String>,

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
