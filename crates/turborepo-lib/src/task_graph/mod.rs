mod visitor;

use std::str::FromStr;

use globwalk::{GlobError, ValidatedGlob};
use serde::{Deserialize, Serialize};
use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_errors::Spanned;
pub use visitor::{Error as VisitorError, Visitor};

use crate::{
    cli::OutputLogsMode,
    run::task_id::{TaskId, TaskName},
    turbo_json::RawTaskDefinition,
};

// TaskOutputs represents the patterns for including and excluding files from
// outputs
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskOutputs {
    pub inclusions: Vec<String>,
    pub exclusions: Vec<String>,
}

impl TaskOutputs {
    pub fn validated_inclusions(&self) -> Result<Vec<ValidatedGlob>, GlobError> {
        self.inclusions
            .iter()
            .map(|i| ValidatedGlob::from_str(i))
            .collect()
    }

    pub fn validated_exclusions(&self) -> Result<Vec<ValidatedGlob>, GlobError> {
        self.exclusions
            .iter()
            .map(|e| ValidatedGlob::from_str(e))
            .collect()
    }
}

// Constructed from a RawTaskDefinition
#[derive(Debug, PartialEq, Clone, Eq)]
pub struct TaskDefinition {
    pub outputs: TaskOutputs,
    pub(crate) cache: bool,

    // This field is custom-marshalled from `env` and `depends_on``
    pub(crate) env: Vec<String>,

    pub(crate) pass_through_env: Option<Vec<String>>,

    // TopologicalDependencies are tasks from package dependencies.
    // E.g. "build" is a topological dependency in:
    // dependsOn: ['^build'].
    // This field is custom-marshalled from rawTask.DependsOn
    pub topological_dependencies: Vec<Spanned<TaskName<'static>>>,

    // TaskDependencies are anything that is not a topological dependency
    // E.g. both something and //whatever are TaskDependencies in:
    // dependsOn: ['something', '//whatever']
    // This field is custom-marshalled from rawTask.DependsOn
    pub task_dependencies: Vec<Spanned<TaskName<'static>>>,

    // Inputs indicate the list of files this Task depends on. If any of those files change
    // we can conclude that any cached outputs or logs for this Task should be invalidated.
    pub(crate) inputs: Vec<String>,

    // OutputMode determines how we should log the output.
    pub(crate) output_logs: OutputLogsMode,

    // Persistent indicates whether the Task is expected to exit or not
    // Tasks marked Persistent do not exit (e.g. --watch mode or dev servers)
    pub persistent: bool,

    // Interactive marks that a task can have it's stdin written to.
    // Tasks that take stdin input cannot be cached as their outputs may depend on the
    // input.
    pub interactive: bool,
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
            output_logs: Default::default(),
            persistent: Default::default(),
            interactive: Default::default(),
        }
    }
}

impl FromIterator<RawTaskDefinition> for RawTaskDefinition {
    fn from_iter<T: IntoIterator<Item = RawTaskDefinition>>(iter: T) -> Self {
        iter.into_iter()
            .fold(RawTaskDefinition::default(), |mut def, other| {
                def.merge(other);
                def
            })
    }
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
}

fn task_log_filename(task_name: &str) -> String {
    format!("turbo-{}.log", task_name.replace(':', "$colon$"))
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
        let build_expected =
            AnchoredSystemPathBuf::from_raw([".turbo", "turbo-build.log"].join(MAIN_SEPARATOR_STR))
                .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log = TaskDefinition::workspace_relative_log_file("build:prod");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            [".turbo", "turbo-build$colon$prod.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log = TaskDefinition::workspace_relative_log_file("build:prod:extra");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            [".turbo", "turbo-build$colon$prod$colon$extra.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);
    }
}
