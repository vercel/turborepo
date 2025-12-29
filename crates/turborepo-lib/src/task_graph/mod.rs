mod visitor;

use std::str::FromStr;

use globwalk::{GlobError, ValidatedGlob};
use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_task_id::TaskId;
// Re-export TaskDefinition from turborepo-types for backward compatibility.
// New code should import directly from `turborepo_types::TaskDefinition`.
pub use turborepo_types::TaskDefinition;
// Re-export TaskInputs from turborepo-types for backward compatibility.
// New code should import directly from `turborepo_types::TaskInputs`.
#[deprecated(
    since = "2.4.0",
    note = "Import `TaskInputs` directly from `turborepo_types` instead"
)]
#[allow(unused_imports)]
pub use turborepo_types::TaskInputs;
// Re-export TaskOutputs from turborepo-types for backward compatibility.
// New code should import directly from `turborepo_types::TaskOutputs`.
#[deprecated(
    since = "2.4.0",
    note = "Import `TaskOutputs` directly from `turborepo_types` instead"
)]
pub use turborepo_types::TaskOutputs;
// Re-export log file utilities from turborepo-types for backward compatibility
pub use turborepo_types::{task_log_filename, LOG_DIR};
pub use visitor::{Error as VisitorError, Visitor};

/// Extension trait for TaskDefinition providing path and output methods.
pub trait TaskDefinitionExt {
    /// Get the workspace-relative path to the log file for a task
    fn workspace_relative_log_file(task_name: &str) -> AnchoredSystemPathBuf;

    /// Get the hashable outputs for a task, including the log file
    fn hashable_outputs(&self, task_name: &TaskId) -> TaskOutputs;

    /// Get the repo-relative hashable outputs for a task
    fn repo_relative_hashable_outputs(
        &self,
        task_name: &TaskId,
        workspace_dir: &AnchoredSystemPath,
    ) -> TaskOutputs;
}

impl TaskDefinitionExt for TaskDefinition {
    fn workspace_relative_log_file(task_name: &str) -> AnchoredSystemPathBuf {
        let log_dir = AnchoredSystemPath::new(LOG_DIR)
            .expect("LOG_DIR should be a valid AnchoredSystemPathBuf");
        log_dir.join_component(&task_log_filename(task_name))
    }

    fn hashable_outputs(&self, task_name: &TaskId) -> TaskOutputs {
        // Delegate to the canonical implementation in turborepo-types
        TaskDefinition::hashable_outputs(self, task_name.task())
    }

    fn repo_relative_hashable_outputs(
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
        let mut repo_relative_globs = TaskDefinitionExt::hashable_outputs(self, task_name);

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

/// Extension trait for TaskOutputs providing glob validation methods.
pub trait TaskOutputsExt {
    /// We consider an empty outputs to be a log output and nothing else
    fn is_empty(&self) -> bool;

    fn validated_inclusions(&self) -> Result<Vec<ValidatedGlob>, GlobError>;

    fn validated_exclusions(&self) -> Result<Vec<ValidatedGlob>, GlobError>;
}

impl TaskOutputsExt for TaskOutputs {
    fn is_empty(&self) -> bool {
        self.inclusions.len() == 1
            && self.inclusions[0].ends_with(".log")
            && self.exclusions.is_empty()
    }

    fn validated_inclusions(&self) -> Result<Vec<ValidatedGlob>, GlobError> {
        self.inclusions
            .iter()
            .map(|i| ValidatedGlob::from_str(i))
            .collect()
    }

    fn validated_exclusions(&self) -> Result<Vec<ValidatedGlob>, GlobError> {
        self.exclusions
            .iter()
            .map(|e| ValidatedGlob::from_str(e))
            .collect()
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
        let build_log = <TaskDefinition as TaskDefinitionExt>::workspace_relative_log_file("build");
        let build_expected =
            AnchoredSystemPathBuf::from_raw([".turbo", "turbo-build.log"].join(MAIN_SEPARATOR_STR))
                .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log =
            <TaskDefinition as TaskDefinitionExt>::workspace_relative_log_file("build:prod");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            [".turbo", "turbo-build$colon$prod.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);

        let build_log =
            <TaskDefinition as TaskDefinitionExt>::workspace_relative_log_file("build:prod:extra");
        let build_expected = AnchoredSystemPathBuf::from_raw(
            [".turbo", "turbo-build$colon$prod$colon$extra.log"].join(MAIN_SEPARATOR_STR),
        )
        .unwrap();
        assert_eq!(build_log, build_expected);
    }
}
