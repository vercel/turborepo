mod visitor;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};
pub use visitor::{Error as VisitorError, Visitor};

use crate::{
    cli::OutputLogsMode,
    run::task_id::{TaskId, TaskName},
};

pub type Pipeline = HashMap<TaskName<'static>, BookkeepingTaskDefinition>;

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct BookkeepingTaskDefinition {
    pub defined_fields: HashSet<String>,
    pub experimental_fields: HashSet<String>,
    pub experimental: TaskDefinitionExperiments,
    pub task_definition: TaskDefinitionStable,
}

// A list of config fields in a task definition that are considered
// experimental. We keep these separated so we can compute a global hash without
// these.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDefinitionExperiments {}

// TaskOutputs represents the patterns for including and excluding files from
// outputs
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskOutputs {
    pub inclusions: Vec<String>,
    pub exclusions: Vec<String>,
}

// These are the stable fields of a TaskDefinition, versus the experimental ones
// TODO: Consolidate this and experiments, because the split is an artifact of
// the Go implementation
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TaskDefinitionStable {
    pub(crate) outputs: TaskOutputs,
    pub(crate) cache: bool,
    pub(crate) topological_dependencies: Vec<TaskName<'static>>,
    pub(crate) task_dependencies: Vec<TaskName<'static>>,
    pub(crate) inputs: Vec<String>,
    pub(crate) output_mode: OutputLogsMode,
    pub(crate) persistent: bool,
    pub(crate) env: Vec<String>,
    pub(crate) pass_through_env: Option<Vec<String>>,
    pub(crate) dot_env: Option<Vec<RelativeUnixPathBuf>>,
}

impl Default for TaskDefinitionStable {
    fn default() -> Self {
        Self {
            outputs: TaskOutputs::default(),
            cache: true,
            topological_dependencies: Vec::new(),
            task_dependencies: Vec::new(),
            inputs: Vec::new(),
            output_mode: OutputLogsMode::default(),
            persistent: false,
            env: Vec::new(),
            pass_through_env: None,
            dot_env: None,
        }
    }
}

// Constructed from a RawTaskDefinition
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct TaskDefinition {
    pub outputs: TaskOutputs,
    pub(crate) cache: bool,

    // This field is custom-marshalled from `env` and `depends_on``
    pub(crate) env: Vec<String>,

    pub(crate) pass_through_env: Option<Vec<String>>,

    pub(crate) dot_env: Option<Vec<RelativeUnixPathBuf>>,

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
    pub(crate) inputs: Vec<String>,

    // OutputMode determines how we should log the output.
    pub(crate) output_mode: OutputLogsMode,

    // Persistent indicates whether the Task is expected to exit or not
    // Tasks marked Persistent do not exit (e.g. --watch mode or dev servers)
    pub persistent: bool,
}

impl BookkeepingTaskDefinition {
    // Useful for splitting out the metadata vs fields which allows for easier
    // definition merging
    fn split(self) -> (Bookkeeping, TaskDefinitionStable, TaskDefinitionExperiments) {
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

const LOG_DIR: &str = ".turbo";

impl TaskDefinition {
    pub fn workspace_relative_log_file(task_name: &str) -> AnchoredSystemPathBuf {
        let log_dir = AnchoredSystemPath::new(LOG_DIR)
            .expect("LOG_DIR should be a valid AnchoredSystemPathBuf");
        log_dir.join_component(&task_log_filename(task_name))
    }

    fn sharable_workspace_relative_log_file(task_name: &str) -> RelativeUnixPathBuf {
        let log_dir = RelativeUnixPathBuf::new(LOG_DIR)
            .expect("LOG_DIR should be a valid relative unix path");
        log_dir.join_component(&task_log_filename(task_name))
    }

    pub fn hashable_outputs(&self, task_name: &TaskId) -> TaskOutputs {
        let mut inclusion_outputs =
            vec![Self::sharable_workspace_relative_log_file(task_name.task()).to_string()];
        inclusion_outputs.extend_from_slice(&self.outputs.inclusions[..]);

        let mut hashable = TaskOutputs {
            inclusions: inclusion_outputs,
            exclusions: self.outputs.exclusions.clone(),
        };

        hashable.inclusions.sort();
        hashable.exclusions.sort();

        hashable
    }

    pub fn repo_relative_hashable_outputs(
        &self,
        task_name: &TaskId,
        workspace_dir: &AnchoredSystemPath,
    ) -> TaskOutputs {
        let make_glob_repo_relative = |glob: &str| -> String {
            let mut repo_relative_glob = workspace_dir.to_string();
            repo_relative_glob.push(std::path::MAIN_SEPARATOR);
            repo_relative_glob.push_str(glob);
            repo_relative_glob
        };

        // At this point repo_relative_globs are still workspace relative, but
        // the processing in the rest of the function converts this to be repo
        // relative.
        let mut repo_relative_globs = self.hashable_outputs(task_name);

        for input in repo_relative_globs.inclusions.iter_mut() {
            let relative_input = make_glob_repo_relative(input.as_str());
            *input = relative_input;
        }

        for output in repo_relative_globs.exclusions.iter_mut() {
            let relative_output = make_glob_repo_relative(output.as_str());
            *output = relative_output;
        }

        repo_relative_globs
    }

    // merge accepts a BookkeepingTaskDefinitions and
    // merges it into TaskDefinition. It uses the bookkeeping
    // defined_fields to determine which fields should be overwritten and when
    // 0-values should be respected.
    pub fn merge(&mut self, other: BookkeepingTaskDefinition) {
        // TODO(olszewski) simplify this construction and throw off the shackles of Go
        let (
            meta,
            TaskDefinitionStable {
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

fn task_log_filename(task_name: &str) -> String {
    format!("turbo-{}.log", task_name.replace(':', "$colon$"))
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

#[cfg(test)]
mod test {
    use std::path::MAIN_SEPARATOR_STR;

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_relative_output_globs() {
        let task_defn = TaskDefinition {
            outputs: TaskOutputs {
                inclusions: vec![".next/**/*".to_string()],
                exclusions: vec![".next/bad-file".to_string()],
            },
            ..Default::default()
        };

        let task_id = TaskId::new("foo", "build");
        let workspace_dir = AnchoredSystemPath::new(match cfg!(windows) {
            true => "apps\\foo",
            false => "apps/foo",
        })
        .unwrap();

        let relative_outputs = task_defn.repo_relative_hashable_outputs(&task_id, workspace_dir);
        let relative_prefix = match cfg!(windows) {
            true => "apps\\foo\\",
            false => "apps/foo/",
        };
        assert_eq!(
            relative_outputs,
            TaskOutputs {
                inclusions: vec![
                    format!("{relative_prefix}.next/**/*"),
                    format!("{relative_prefix}.turbo/turbo-build.log"),
                ],
                exclusions: vec![format!("{relative_prefix}.next/bad-file")],
            }
        );
    }

    #[test]
    fn test_escape_log_file() {
        let build_log = TaskDefinition::workspace_relative_log_file("build");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            &[".turbo", "turbo-build.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log = TaskDefinition::workspace_relative_log_file("build:prod");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            &[".turbo", "turbo-build$colon$prod.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log = TaskDefinition::workspace_relative_log_file("build:prod:extra");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            &[".turbo", "turbo-build$colon$prod$colon$extra.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);
    }
}
