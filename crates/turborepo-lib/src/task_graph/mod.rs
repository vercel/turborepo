use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use turbopath::RelativeUnixPathBuf;

use crate::run::task_id::TaskName;

pub type Pipeline = HashMap<TaskName<'static>, BookkeepingTaskDefinition>;

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
#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskOutputMode {
    // FullTaskOutput will show all task output
    #[default]
    Full,
    // None will hide all task output
    None,
    // Hash will display turbo-computed task hashes
    HashOnly,
    // New will show all new task output and turbo-computed task hashes for cached
    // output
    NewOnly,
    // Error will show task output for failures only; no cache miss/hit messages are
    // emitted
    ErrorsOnly,
}

// taskDefinitionHashable exists as a definition for PristinePipeline, which is
// used downstream for calculating the global hash. We want to exclude
// experimental fields here because we don't want experimental fields to be part
// of the global hash.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TaskDefinitionHashable {
    pub(crate) outputs: TaskOutputs,
    pub(crate) cache: bool,
    pub(crate) topological_dependencies: Vec<TaskName<'static>>,
    pub(crate) task_dependencies: Vec<TaskName<'static>>,
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
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskDefinition {
    outputs: TaskOutputs,
    cache: bool,

    // This field is custom-marshalled from `env` and `depends_on``
    env: Vec<String>,

    pass_through_env: Vec<String>,

    dot_env: Vec<RelativeUnixPathBuf>,

    // TopologicalDependencies are tasks from package dependencies.
    // E.g. "build" is a topological dependency in:
    // dependsOn: ['^build'].
    // This field is custom-marshalled from rawTask.DependsOn
    pub topological_dependencies: Vec<TaskName<'static>>,

    // TaskDependencies are anything that is not a topological dependency
    // E.g. both something and //whatever are TaskDependencies in:
    // dependsOn: ['something', '//whatever']
    // This field is custom-marshalled from rawTask.DependsOn
    pub task_dependencies: Vec<TaskName<'static>>,

    // Inputs indicate the list of files this Task depends on. If any of those files change
    // we can conclude that any cached outputs or logs for this Task should be invalidated.
    inputs: Vec<String>,

    // OutputMode determines how we should log the output.
    output_mode: TaskOutputMode,

    // Persistent indicates whether the Task is expected to exit or not
    // Tasks marked Persistent do not exit (e.g. --watch mode or dev servers)
    persistent: bool,
}

impl BookkeepingTaskDefinition {
    // Useful for splitting out the metadata vs fields which allows for easier
    // definition merging
    fn split(
        self,
    ) -> (
        Bookkeeping,
        TaskDefinitionHashable,
        TaskDefinitionExperiments,
    ) {
        let Self {
            defined_fields,
            experimental_fields,
            experimental,
            task_definition,
        } = self;
        (
            Bookkeeping {
                defined_fields,
                experimental_fields,
            },
            task_definition,
            experimental,
        )
    }
}

struct Bookkeeping {
    defined_fields: HashSet<String>,
    experimental_fields: HashSet<String>,
}

impl Bookkeeping {
    fn has_field(&self, field_name: &str) -> bool {
        self.defined_fields.contains(field_name) || self.experimental_fields.contains(field_name)
    }
}

impl Default for TaskDefinition {
    fn default() -> Self {
        Self {
            cache: true,
            outputs: Default::default(),
            env: Default::default(),
            pass_through_env: Default::default(),
            topological_dependencies: Default::default(),
            task_dependencies: Default::default(),
            inputs: Default::default(),
            output_mode: Default::default(),
            persistent: Default::default(),
            dot_env: Default::default(),
        }
    }
}

macro_rules! set_field {
    ($this:ident, $book:ident, $field:ident) => {{
        if $book.has_field(stringify!($field)) {
            $this.$field = $field;
        }
    }};

    ($this:ident, $book:ident, $field:ident, $field_name:literal) => {{
        if $book.has_field($field_name) {
            $this.$field = $field;
        }
    }};
}

impl TaskDefinition {
    // merge accepts a BookkeepingTaskDefinitions and
    // merges it into TaskDefinition. It uses the bookkeeping
    // defined_fields to determine which fields should be overwritten and when
    // 0-values should be respected.
    pub fn merge(&mut self, other: BookkeepingTaskDefinition) {
        // TODO(olszewski) simplify this construction and throw off the shackles of Go
        let (
            meta,
            TaskDefinitionHashable {
                outputs,
                cache,
                topological_dependencies,
                task_dependencies,
                inputs,
                output_mode,
                persistent,
                env,
                pass_through_env,
                dot_env,
            },
            _experimental,
        ) = other.split();

        set_field!(self, meta, outputs, "Outputs");
        set_field!(self, meta, cache, "Cache");
        set_field!(self, meta, topological_dependencies, "DependsOn");
        set_field!(self, meta, task_dependencies, "DependsOn");
        set_field!(self, meta, inputs, "Inputs");
        set_field!(self, meta, output_mode, "OutputMode");
        set_field!(self, meta, persistent, "Persistent");
        set_field!(self, meta, env, "Env");
        set_field!(self, meta, pass_through_env, "PassThroughEnv");
        set_field!(self, meta, dot_env, "DotEnv");
    }
}

impl FromIterator<BookkeepingTaskDefinition> for TaskDefinition {
    fn from_iter<T: IntoIterator<Item = BookkeepingTaskDefinition>>(iter: T) -> Self {
        iter.into_iter()
            .fold(TaskDefinition::default(), |mut def, other| {
                def.merge(other);
                def
            })
    }
}
