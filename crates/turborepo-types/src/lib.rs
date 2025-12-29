//! Shared types for Turborepo
//!
//! This crate contains types that are used across multiple crates in the
//! turborepo ecosystem. It serves as a foundation layer to avoid circular
//! dependencies between higher-level crates.

use std::fmt;

use biome_deserialize_macros::Deserializable;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use turbopath::RelativeUnixPathBuf;
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;

/// Environment mode for task execution.
///
/// Controls how environment variables are handled during task execution:
/// - `Loose`: Allows all environment variables to be passed through
/// - `Strict`: Only explicitly configured environment variables are passed
///   through
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Serialize, ValueEnum, Deserialize, Eq, Deserializable,
)]
#[serde(rename_all = "lowercase")]
pub enum EnvMode {
    Loose,
    #[default]
    Strict,
}

impl fmt::Display for EnvMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EnvMode::Loose => "loose",
            EnvMode::Strict => "strict",
        })
    }
}

/// Output log mode for task execution.
///
/// Controls how task output logs are displayed and persisted:
/// - `Full`: Entire task output is persisted after run
/// - `None`: None of a task output is persisted after run
/// - `HashOnly`: Only the status line of a task is persisted
/// - `NewOnly`: Output is only persisted if it is a cache miss
/// - `ErrorsOnly`: Output is only persisted if the task failed
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum, Deserializable, Serialize)]
pub enum OutputLogsMode {
    #[serde(rename = "full")]
    #[default]
    Full,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "hash-only")]
    HashOnly,
    #[serde(rename = "new-only")]
    NewOnly,
    #[serde(rename = "errors-only")]
    ErrorsOnly,
}

impl fmt::Display for OutputLogsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputLogsMode::Full => "full",
            OutputLogsMode::None => "none",
            OutputLogsMode::HashOnly => "hash-only",
            OutputLogsMode::NewOnly => "new-only",
            OutputLogsMode::ErrorsOnly => "errors-only",
        })
    }
}

/// TaskOutputs represents the patterns for including and excluding files from
/// outputs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskOutputs {
    pub inclusions: Vec<String>,
    pub exclusions: Vec<String>,
}

/// TaskInputs represents the input file patterns for a task.
///
/// Contains glob patterns for files that the task depends on, and a flag
/// indicating whether to include default inputs ($TURBO_DEFAULT$).
#[derive(Debug, PartialEq, Clone, Eq, Default)]
pub struct TaskInputs {
    /// Glob patterns for input files
    pub globs: Vec<String>,
    /// Set when $TURBO_DEFAULT$ is in inputs
    pub default: bool,
}

impl TaskInputs {
    /// Creates a new TaskInputs with the given globs and default set to false
    pub fn new(globs: Vec<String>) -> Self {
        Self {
            globs,
            default: false,
        }
    }

    /// Sets the default flag and returns self for method chaining
    pub fn with_default(mut self, default: bool) -> Self {
        self.default = default;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_mode_display() {
        assert_eq!(format!("{}", EnvMode::Loose), "loose");
        assert_eq!(format!("{}", EnvMode::Strict), "strict");
    }

    #[test]
    fn output_logs_mode_display() {
        assert_eq!(format!("{}", OutputLogsMode::Full), "full");
        assert_eq!(format!("{}", OutputLogsMode::None), "none");
        assert_eq!(format!("{}", OutputLogsMode::HashOnly), "hash-only");
        assert_eq!(format!("{}", OutputLogsMode::NewOnly), "new-only");
        assert_eq!(format!("{}", OutputLogsMode::ErrorsOnly), "errors-only");
    }

    #[test]
    fn output_logs_mode_default() {
        assert_eq!(OutputLogsMode::default(), OutputLogsMode::Full);
    }

    #[test]
    fn env_mode_default() {
        assert_eq!(EnvMode::default(), EnvMode::Strict);
    }

    #[test]
    fn task_outputs_default() {
        let outputs = TaskOutputs::default();
        assert!(outputs.inclusions.is_empty());
        assert!(outputs.exclusions.is_empty());
    }

    #[test]
    fn task_inputs_default() {
        let inputs = TaskInputs::default();
        assert!(inputs.globs.is_empty());
        assert!(!inputs.default);
    }

    #[test]
    fn task_inputs_new() {
        let inputs = TaskInputs::new(vec!["src/**".to_string()]);
        assert_eq!(inputs.globs, vec!["src/**".to_string()]);
        assert!(!inputs.default);
    }

    #[test]
    fn task_inputs_with_default() {
        let inputs = TaskInputs::new(vec!["src/**".to_string()]).with_default(true);
        assert_eq!(inputs.globs, vec!["src/**".to_string()]);
        assert!(inputs.default);
    }

    #[test]
    fn test_task_log_filename() {
        assert_eq!(task_log_filename("build"), "turbo-build.log");
        assert_eq!(
            task_log_filename("build:prod"),
            "turbo-build$colon$prod.log"
        );
        assert_eq!(
            task_log_filename("build:prod:extra"),
            "turbo-build$colon$prod$colon$extra.log"
        );
    }

    #[test]
    fn test_sharable_workspace_relative_log_file() {
        let path = sharable_workspace_relative_log_file("build");
        assert_eq!(path.as_str(), ".turbo/turbo-build.log");

        let path = sharable_workspace_relative_log_file("build:prod");
        assert_eq!(path.as_str(), ".turbo/turbo-build$colon$prod.log");
    }

    #[test]
    fn test_hashable_outputs_includes_log_file() {
        let task_def = TaskDefinition {
            outputs: TaskOutputs {
                inclusions: vec!["dist/**".to_string()],
                exclusions: vec!["dist/temp".to_string()],
            },
            ..Default::default()
        };

        let result = task_def.hashable_outputs("build");

        // Log file should be included and outputs should be sorted
        assert!(result
            .inclusions
            .contains(&".turbo/turbo-build.log".to_string()));
        assert!(result.inclusions.contains(&"dist/**".to_string()));
        assert_eq!(result.exclusions, vec!["dist/temp".to_string()]);
    }

    #[test]
    fn test_hashable_outputs_sorts_outputs() {
        let task_def = TaskDefinition {
            outputs: TaskOutputs {
                inclusions: vec!["z-output".to_string(), "a-output".to_string()],
                exclusions: vec!["z-exclude".to_string(), "a-exclude".to_string()],
            },
            ..Default::default()
        };

        let result = task_def.hashable_outputs("build");

        // Should be sorted
        assert_eq!(
            result.inclusions,
            vec![
                ".turbo/turbo-build.log".to_string(),
                "a-output".to_string(),
                "z-output".to_string(),
            ]
        );
        assert_eq!(
            result.exclusions,
            vec!["a-exclude".to_string(), "z-exclude".to_string(),]
        );
    }

    #[test]
    fn test_hashable_outputs_escapes_colons() {
        let task_def = TaskDefinition::default();
        let result = task_def.hashable_outputs("build:prod");

        assert!(result
            .inclusions
            .contains(&".turbo/turbo-build$colon$prod.log".to_string()));
    }
}

/// Constructed from a RawTaskDefinition, this represents the fully resolved
/// configuration for a task.
#[derive(Debug, PartialEq, Clone, Eq)]
pub struct TaskDefinition {
    pub outputs: TaskOutputs,
    pub cache: bool,

    // This field is custom-marshalled from `env` and `depends_on`
    pub env: Vec<String>,

    pub pass_through_env: Option<Vec<String>>,

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
    pub inputs: TaskInputs,

    // OutputMode determines how we should log the output.
    pub output_logs: OutputLogsMode,

    // Persistent indicates whether the Task is expected to exit or not
    // Tasks marked Persistent do not exit (e.g. watch mode or dev servers)
    pub persistent: bool,

    // Indicates whether a persistent task can be interrupted in the middle of execution
    // by watch mode
    pub interruptible: bool,

    // Interactive marks that a task can have its stdin written to.
    // Tasks that take stdin input cannot be cached as their outputs may depend on the
    // input.
    pub interactive: bool,

    // Override for global env mode setting
    pub env_mode: Option<EnvMode>,

    // Tasks that will get added to the graph if this one is
    // It contains no guarantees regarding ordering, just that this will also get run.
    // It will also not affect the task's hash aside from the definition getting folded into the
    // hash.
    pub with: Option<Vec<Spanned<TaskName<'static>>>>,
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
            interruptible: Default::default(),
            interactive: Default::default(),
            env_mode: Default::default(),
            with: Default::default(),
        }
    }
}

/// Directory where turbo stores task logs
pub const LOG_DIR: &str = ".turbo";

/// Generate the log filename for a task, escaping colons in the task name.
///
/// # Example
/// ```
/// use turborepo_types::task_log_filename;
/// assert_eq!(task_log_filename("build"), "turbo-build.log");
/// assert_eq!(task_log_filename("build:prod"), "turbo-build$colon$prod.log");
/// ```
pub fn task_log_filename(task_name: &str) -> String {
    format!("turbo-{}.log", task_name.replace(':', "$colon$"))
}

/// Get the workspace-relative path to the log file for a task as a
/// `RelativeUnixPathBuf`. This is used for cache hash computation and is
/// platform-independent.
///
/// # Example
/// ```
/// use turborepo_types::sharable_workspace_relative_log_file;
/// let path = sharable_workspace_relative_log_file("build");
/// assert_eq!(path.as_str(), ".turbo/turbo-build.log");
/// ```
pub fn sharable_workspace_relative_log_file(task_name: &str) -> RelativeUnixPathBuf {
    let log_dir =
        RelativeUnixPathBuf::new(LOG_DIR).expect("LOG_DIR should be a valid relative unix path");
    log_dir.join_component(&task_log_filename(task_name))
}

impl TaskDefinition {
    /// Returns the hashable outputs for this task, including the log file.
    ///
    /// This is the canonical implementation used for cache key computation.
    /// The outputs are sorted to ensure deterministic hash computation.
    ///
    /// # Arguments
    /// * `task_name` - The task name (e.g., "build" or "build:prod")
    ///
    /// # Returns
    /// A `TaskOutputs` with:
    /// - The log file path prepended to inclusions
    /// - All inclusions sorted
    /// - All exclusions sorted
    pub fn hashable_outputs(&self, task_name: &str) -> TaskOutputs {
        let mut inclusion_outputs =
            vec![sharable_workspace_relative_log_file(task_name).to_string()];
        inclusion_outputs.extend_from_slice(&self.outputs.inclusions[..]);

        let mut hashable = TaskOutputs {
            inclusions: inclusion_outputs,
            exclusions: self.outputs.exclusions.clone(),
        };

        hashable.inclusions.sort();
        hashable.exclusions.sort();

        hashable
    }
}
