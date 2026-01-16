//! Shared types for Turborepo
//!
//! This crate contains types that are used across multiple crates in the
//! turborepo ecosystem. It serves as a foundation layer to avoid circular
//! dependencies between higher-level crates.
//!
//! # Traits for Cross-Crate Abstraction
//!
//! This crate defines traits that allow higher-level crates to depend on
//! abstractions rather than concrete implementations:
//!
//! - [`EngineInfo`]: Provides access to task definitions and dependencies
//! - [`RunOptsInfo`]: Provides access to run options
//! - [`HashTrackerInfo`]: Provides access to task hash information
//! - [`GlobalHashInputs`]: Provides access to global hash inputs

use std::{collections::HashMap, fmt, str::FromStr};

use biome_deserialize_macros::Deserializable;
use clap::ValueEnum;
use globwalk::{GlobError, ValidatedGlob};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use turbopath::{
    AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf,
};
use turborepo_errors::Spanned;
use turborepo_task_id::{TaskId, TaskName};

/// Turborepo's Environment Modes allow you to control which environment
/// variables are available to a task at runtime.
///
/// - `strict`: Filter environment variables to only those that are specified in
///   the `env` and `globalEnv` keys in `turbo.json`.
/// - `loose`: Allow all environment variables for the process to be available.
///
/// Documentation: https://turborepo.dev/docs/reference/configuration#envmode
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    PartialEq,
    Serialize,
    ValueEnum,
    Deserialize,
    Eq,
    Deserializable,
    JsonSchema,
    TS,
)]
#[serde(rename_all = "lowercase")]
#[schemars(rename_all = "lowercase")]
#[ts(export)]
pub enum EnvMode {
    /// Allow all environment variables for the process to be available.
    Loose,
    /// Filter environment variables to only those that are specified in the
    /// `env` and `globalEnv` keys in `turbo.json`.
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

/// Signals that task execution should stop.
///
/// This is used to communicate back to the engine whether dependent tasks
/// should continue or stop after a task completes or fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopExecution {
    /// Stop all tasks (used for hard failures)
    AllTasks,
    /// Stop only dependent tasks (used for soft failures with
    /// continue-on-error)
    DependentTasks,
}

/// Output mode for the task.
///
/// - `full`: Displays all output
/// - `hash-only`: Show only the hashes of the tasks
/// - `new-only`: Only show output from cache misses
/// - `errors-only`: Only show output from task failures
/// - `none`: Hides all task output
///
/// Documentation: https://turborepo.dev/docs/reference/run#--output-logs-option
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum, Deserializable, Serialize, JsonSchema, TS,
)]
#[schemars(rename = "OutputLogs")]
#[ts(export, rename = "OutputLogs")]
pub enum OutputLogsMode {
    /// Displays all output.
    #[serde(rename = "full")]
    #[default]
    Full,
    /// Hides all task output.
    #[serde(rename = "none")]
    None,
    /// Show only the hashes of the tasks.
    #[serde(rename = "hash-only")]
    HashOnly,
    /// Only show output from cache misses.
    #[serde(rename = "new-only")]
    NewOnly,
    /// Only show output from task failures.
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

/// Dry run output mode.
///
/// Controls the format of dry run output:
/// - `Text`: Human-readable text output
/// - `Json`: Machine-readable JSON output
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum, Serialize)]
pub enum DryRunMode {
    Text,
    Json,
}

impl fmt::Display for DryRunMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DryRunMode::Text => "text",
            DryRunMode::Json => "json",
        })
    }
}

/// Enable use of the UI for `turbo`.
///
/// Documentation: https://turborepo.dev/docs/reference/configuration#ui
#[derive(
    Serialize,
    Deserialize,
    Debug,
    Default,
    Copy,
    Clone,
    Deserializable,
    PartialEq,
    Eq,
    ValueEnum,
    JsonSchema,
    TS,
)]
#[serde(rename_all = "camelCase")]
#[schemars(rename = "UI", rename_all = "camelCase")]
#[ts(export, rename = "UI")]
pub enum UIMode {
    /// Use the terminal user interface.
    #[default]
    Tui,
    /// Use the standard output stream.
    Stream,
    /// Use the web user interface.
    /// Note: This feature is undocumented, experimental, and not meant to be
    /// used. It may change or be removed at any time.
    #[schemars(skip)]
    #[ts(skip)]
    Web,
}

impl fmt::Display for UIMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UIMode::Tui => write!(f, "tui"),
            UIMode::Stream => write!(f, "stream"),
            UIMode::Web => write!(f, "web"),
        }
    }
}

impl UIMode {
    pub fn use_tui(&self) -> bool {
        matches!(self, Self::Tui)
    }

    /// Returns true if the UI mode has a sender,
    /// i.e. web or tui but not stream
    pub fn has_sender(&self) -> bool {
        matches!(self, Self::Tui | Self::Web)
    }
}

/// Log ordering mode for task output.
///
/// Controls the order in which task logs are displayed:
/// - `Auto`: System decides based on context
/// - `Stream`: Logs are streamed as they arrive
/// - `Grouped`: Logs are grouped by task
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, ValueEnum, Deserialize, Eq)]
pub enum LogOrder {
    #[serde(rename = "auto")]
    #[default]
    Auto,
    #[serde(rename = "stream")]
    Stream,
    #[serde(rename = "grouped")]
    Grouped,
}

impl fmt::Display for LogOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LogOrder::Auto => "auto",
            LogOrder::Stream => "stream",
            LogOrder::Grouped => "grouped",
        })
    }
}

impl LogOrder {
    pub fn compatible_with_tui(&self) -> bool {
        // If the user requested a specific order to the logs, then this isn't
        // compatible with the TUI and means we cannot use it.
        matches!(self, Self::Auto)
    }
}

/// Continue mode for task execution.
///
/// Controls how task execution continues after failures:
/// - `Never`: Stop on first failure
/// - `DependenciesSuccessful`: Continue if dependencies succeeded
/// - `Always`: Always continue regardless of failures
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContinueMode {
    #[default]
    Never,
    DependenciesSuccessful,
    Always,
}

impl fmt::Display for ContinueMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ContinueMode::Never => "never",
            ContinueMode::DependenciesSuccessful => "dependencies-successful",
            ContinueMode::Always => "always",
        })
    }
}

/// Log prefix mode for task output.
///
/// Controls how task output lines are prefixed:
/// - `Auto`: System decides based on context
/// - `None`: No prefix
/// - `Task`: Prefix with task name
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum, Serialize)]
pub enum LogPrefix {
    #[serde(rename = "auto")]
    #[default]
    Auto,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "task")]
    Task,
}

impl fmt::Display for LogPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LogPrefix::Auto => "auto",
            LogPrefix::None => "none",
            LogPrefix::Task => "task",
        })
    }
}

/// Resolved log ordering mode for task output.
///
/// This is the resolved version of [`LogOrder`] after the `Auto` variant
/// has been resolved to a concrete value based on the execution context.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum ResolvedLogOrder {
    Stream,
    Grouped,
}

/// Resolved log prefix mode for task output.
///
/// This is the resolved version of [`LogPrefix`] after the `Auto` variant
/// has been resolved to a concrete value based on the execution context.
#[derive(Debug, Clone, Copy, Serialize)]
pub enum ResolvedLogPrefix {
    Task,
    None,
}

impl From<LogPrefix> for ResolvedLogPrefix {
    fn from(value: LogPrefix) -> Self {
        match value {
            // We default to task-prefixed logs
            LogPrefix::Auto | LogPrefix::Task => ResolvedLogPrefix::Task,
            LogPrefix::None => ResolvedLogPrefix::None,
        }
    }
}

/// Options for graph output.
///
/// Controls where the task graph is written:
/// - `Stdout`: Print to standard output
/// - `File`: Write to the specified file path
#[derive(Clone, Debug, Serialize)]
pub enum GraphOpts {
    Stdout,
    File(String),
}

/// API client configuration options.
///
/// Contains all settings needed to connect to the Turborepo remote cache API,
/// including authentication, timeouts, and server URLs.
#[derive(Debug, Clone, Serialize)]
pub struct APIClientOpts {
    /// Base URL for the Turborepo API
    pub api_url: String,
    /// Request timeout in seconds
    pub timeout: u64,
    /// Upload-specific timeout in seconds
    pub upload_timeout: u64,
    /// Authentication token (if authenticated)
    pub token: Option<String>,
    /// Team ID for the remote cache
    pub team_id: Option<String>,
    /// Team slug for the remote cache
    pub team_slug: Option<String>,
    /// Login URL for authentication flow
    pub login_url: String,
    /// Whether to use preflight requests
    pub preflight: bool,
    /// Port for SSO login callback
    pub sso_login_callback_port: Option<u16>,
}

/// Repository options.
///
/// Contains settings that control how Turborepo interacts with the repository,
/// including configuration file paths and package manager requirements.
#[derive(Debug, Clone, Serialize)]
pub struct RepoOpts {
    /// Path to the root turbo.json configuration file
    pub root_turbo_json_path: AbsoluteSystemPathBuf,
    /// Allow running without a package manager
    pub allow_no_package_manager: bool,
    /// Allow running without a turbo.json file
    pub allow_no_turbo_json: bool,
}

/// TUI (Terminal User Interface) options.
///
/// Contains settings for the terminal UI display.
#[derive(Clone, Debug, Serialize)]
pub struct TuiOpts {
    /// Number of lines to keep in the scrollback buffer
    pub scrollback_length: u64,
}

/// Options for configuring the run cache behavior.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct RunCacheOpts {
    /// Override for task output logs mode
    pub task_output_logs_override: Option<OutputLogsMode>,
    /// When using `outputLogs: "errors-only"`, show task hashes when tasks
    /// complete successfully. Controlled by the `errorsOnlyShowHash` future
    /// flag.
    pub errors_only_show_hash: bool,
}

/// Options for scope resolution.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScopeOpts {
    /// Root for package inference (from cwd)
    pub pkg_inference_root: Option<AnchoredSystemPathBuf>,
    /// Global dependencies that affect all packages
    pub global_deps: Vec<String>,
    /// Filter patterns from --filter flag
    pub filter_patterns: Vec<String>,
    /// Git range for affected detection (from_ref, to_ref)
    pub affected_range: Option<(Option<String>, Option<String>)>,
}

impl ScopeOpts {
    /// Get the filter patterns.
    pub fn get_filters(&self) -> Vec<String> {
        self.filter_patterns.clone()
    }
}

/// Projection of run options that only includes information necessary to
/// compute pass through args for tasks.
///
/// This struct provides a lightweight view into the run configuration
/// specifically for determining which arguments should be passed through
/// to individual tasks.
#[derive(Debug)]
pub struct TaskArgs<'a> {
    pass_through_args: &'a [String],
    tasks: &'a [String],
}

impl<'a> TaskArgs<'a> {
    /// Creates a new TaskArgs instance.
    pub fn new(pass_through_args: &'a [String], tasks: &'a [String]) -> Self {
        TaskArgs {
            pass_through_args,
            tasks,
        }
    }

    /// Returns the pass-through arguments for a specific task if applicable.
    ///
    /// Arguments are returned if:
    /// 1. There are pass-through arguments configured
    /// 2. The task is one of the explicitly requested tasks
    ///
    /// # Arguments
    /// * `task_id` - The task ID to get arguments for
    ///
    /// # Returns
    /// `Some(&[String])` if arguments should be passed through, `None`
    /// otherwise
    pub fn args_for_task(&self, task_id: &TaskId) -> Option<&'a [String]> {
        if !self.pass_through_args.is_empty()
            && self
                .tasks
                .iter()
                .any(|task| TaskName::from(task.as_str()).task() == task_id.task())
        {
            Some(self.pass_through_args)
        } else {
            None
        }
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

    #[test]
    fn test_task_definition_ext_workspace_relative_log_file() {
        let path = TaskDefinition::workspace_relative_log_file("build");
        #[cfg(not(windows))]
        assert_eq!(path.as_str(), ".turbo/turbo-build.log");
        #[cfg(windows)]
        assert_eq!(path.as_str(), ".turbo\\turbo-build.log");

        let path = TaskDefinition::workspace_relative_log_file("build:prod");
        #[cfg(not(windows))]
        assert_eq!(path.as_str(), ".turbo/turbo-build$colon$prod.log");
        #[cfg(windows)]
        assert_eq!(path.as_str(), ".turbo\\turbo-build$colon$prod.log");
    }

    #[test]
    fn test_task_outputs_ext_is_empty() {
        // Empty outputs (only log file)
        let outputs = TaskOutputs {
            inclusions: vec![".turbo/turbo-build.log".to_string()],
            exclusions: vec![],
        };
        assert!(outputs.is_empty());

        // Non-empty outputs
        let outputs = TaskOutputs {
            inclusions: vec![".turbo/turbo-build.log".to_string(), "dist/**".to_string()],
            exclusions: vec![],
        };
        assert!(!outputs.is_empty());

        // Has exclusions
        let outputs = TaskOutputs {
            inclusions: vec![".turbo/turbo-build.log".to_string()],
            exclusions: vec!["node_modules".to_string()],
        };
        assert!(!outputs.is_empty());
    }

    #[test]
    fn test_task_outputs_ext_validated_globs() {
        let outputs = TaskOutputs {
            inclusions: vec!["dist/**".to_string(), "build/**/*.js".to_string()],
            exclusions: vec!["dist/temp".to_string()],
        };

        let inclusions = outputs.validated_inclusions().unwrap();
        assert_eq!(inclusions.len(), 2);

        let exclusions = outputs.validated_exclusions().unwrap();
        assert_eq!(exclusions.len(), 1);
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

// ============================================================================
// Extension Traits for TaskDefinition and TaskOutputs
// ============================================================================
//
// These extension traits provide additional functionality for TaskDefinition
// and TaskOutputs that requires dependencies not available in the core type
// definitions. They are defined here in turborepo-types so they can be used
// across multiple crates.

/// Extension trait for TaskDefinition providing path and output methods.
pub trait TaskDefinitionExt {
    /// Get the workspace-relative path to the log file for a task
    fn workspace_relative_log_file(task_name: &str) -> AnchoredSystemPathBuf;

    /// Get the hashable outputs for a task (delegates to the inherent method)
    fn hashable_outputs_for_task(&self, task_name: &TaskId) -> TaskOutputs;

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

    fn hashable_outputs_for_task(&self, task_name: &TaskId) -> TaskOutputs {
        // Delegate to the canonical implementation in TaskDefinition
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
        let mut repo_relative_globs = self.hashable_outputs_for_task(task_name);

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

// ============================================================================
// Cross-Crate Abstraction Traits
// ============================================================================
//
// These traits define interfaces that allow higher-level crates (like
// turborepo-run-summary) to depend on abstractions rather than concrete
// implementations from lower-level crates (like turborepo-engine,
// turborepo-task-hash). This enables proper dependency direction where
// infrastructure crates don't need to depend on reporting/presentation crates.

/// Trait for accessing engine information (task definitions, dependencies).
///
/// This trait abstracts the task execution engine to allow crates like
/// `turborepo-run-summary` to generate summaries without depending on the
/// full engine implementation.
///
/// # Implementors
/// - `Engine<Built, TaskDefinition>` from `turborepo-engine`
///
/// # Associated Types
/// - `TaskIter`: An iterator over task IDs for dependencies/dependents.
///
/// # Example
/// ```ignore
/// impl EngineInfo for MyEngine {
///     type TaskIter<'a> = /* iterator type */;
///     fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&TaskDefinition> { ... }
///     fn dependencies(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> { ... }
///     fn dependents(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>> { ... }
/// }
/// ```
pub trait EngineInfo {
    /// Iterator type for task dependencies/dependents
    type TaskIter<'a>: Iterator<Item = &'a TaskId<'static>>
    where
        Self: 'a;

    /// Returns the task definition for a given task ID
    fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&TaskDefinition>;
    /// Returns an iterator over the task's dependencies (tasks it depends on)
    fn dependencies(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>>;
    /// Returns an iterator over the task's dependents (tasks that depend on it)
    fn dependents(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>>;
}

/// Trait for accessing run options.
///
/// This trait abstracts run configuration to allow summary generation
/// without depending on the full opts implementation.
///
/// # Implementors
/// - `RunOpts` from `turborepo-lib`
pub trait RunOptsInfo {
    /// Returns the dry run mode if running in dry mode, None otherwise
    fn dry_run(&self) -> Option<DryRunMode>;
    /// Returns true if this is a single-package (non-monorepo) run
    fn single_package(&self) -> bool;
    /// Returns Some("true") if run summary should be saved to disk
    fn summarize(&self) -> Option<&str>;
    /// Returns true if framework detection is enabled
    fn framework_inference(&self) -> bool;
    /// Returns arguments to pass through to task execution
    fn pass_through_args(&self) -> &[String];
    /// Returns the list of task names being run
    fn tasks(&self) -> &[String];
}

/// Trait for accessing task hash tracking information.
///
/// Provides access to computed hashes, environment variables, cache status,
/// and expanded inputs/outputs for tasks during run summary generation.
///
/// # Implementors
/// - `TaskHashTracker` from `turborepo-task-hash`
///
/// # Note
/// The `DetailedMap` type is from `turborepo-env`. Implementors should
/// re-export or use a compatible type.
pub trait HashTrackerInfo {
    /// Returns the computed hash for a task
    fn hash(&self, task_id: &TaskId) -> Option<String>;
    /// Returns the detailed environment variable map for a task
    fn env_vars(&self, task_id: &TaskId) -> Option<HashTrackerDetailedMap>;
    /// Returns the cache hit metadata for a task
    fn cache_status(&self, task_id: &TaskId) -> Option<HashTrackerCacheHitMetadata>;
    /// Returns the expanded output paths for a task
    fn expanded_outputs(&self, task_id: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>>;
    /// Returns the detected framework for a task
    fn framework(&self, task_id: &TaskId) -> Option<String>;
    /// Returns the expanded input file hashes for a task
    fn expanded_inputs(&self, task_id: &TaskId) -> Option<HashMap<RelativeUnixPathBuf, String>>;
}

/// Detailed environment variable map for hash tracking.
///
/// This is a type alias placeholder - actual implementations should use
/// their crate's DetailedMap type.
#[derive(Debug, Clone, Default)]
pub struct HashTrackerDetailedMap {
    /// Environment variables from explicit configuration
    pub explicit: Vec<String>,
    /// Environment variables from framework inference
    pub matching: Vec<String>,
}

/// Cache hit metadata for hash tracking.
///
/// Indicates where a cache hit was found (local or remote).
#[derive(Debug, Clone)]
pub struct HashTrackerCacheHitMetadata {
    /// Whether the cache hit was from local cache
    pub local: bool,
    /// Whether the cache hit was from remote cache
    pub remote: bool,
    /// Time saved by the cache hit in milliseconds
    pub time_saved: u64,
}

/// Trait for types that provide task definition information needed for hashing.
///
/// This allows task_hash to be decoupled from the full TaskDefinition type
/// while still having access to the fields it needs for hash computation.
///
/// # Implementors
/// - `TaskDefinition` (implemented below)
pub trait TaskDefinitionHashInfo {
    /// Returns the list of environment variable patterns for this task
    fn env(&self) -> &[String];
    /// Returns the pass-through environment variables
    fn pass_through_env(&self) -> Option<&[String]>;
    /// Returns the task inputs configuration
    fn inputs(&self) -> &TaskInputs;
    /// Returns the task outputs configuration
    fn outputs(&self) -> &TaskOutputs;
    /// Returns the hashable outputs for this task (includes log file)
    fn hashable_outputs(&self, task_id: &TaskId) -> TaskOutputs;
}

impl TaskDefinitionHashInfo for TaskDefinition {
    fn env(&self) -> &[String] {
        &self.env
    }

    fn pass_through_env(&self) -> Option<&[String]> {
        self.pass_through_env.as_deref()
    }

    fn inputs(&self) -> &TaskInputs {
        &self.inputs
    }

    fn outputs(&self) -> &TaskOutputs {
        &self.outputs
    }

    fn hashable_outputs(&self, task_id: &TaskId) -> TaskOutputs {
        // Delegate to the canonical implementation in TaskDefinition
        TaskDefinition::hashable_outputs(self, task_id.task())
    }
}

/// Trait for run options needed by the task hasher.
///
/// This allows task_hash to be decoupled from the full RunOpts type.
///
/// # Implementors
/// - `RunOpts` from `turborepo-lib`
pub trait RunOptsHashInfo {
    /// Whether to infer the framework for each workspace
    fn framework_inference(&self) -> bool;
    /// Whether this is a single-package repo (not a monorepo)
    fn single_package(&self) -> bool;
    /// Arguments to pass through to tasks
    fn pass_through_args(&self) -> &[String];
}

/// Trait for global hash inputs.
///
/// Provides access to global configuration that affects all task hashes.
/// This includes environment variables, file dependencies, and dependency
/// hashes.
///
/// # Implementors
/// - `GlobalHashableInputs` from `turborepo-lib`
pub trait GlobalHashInputs {
    /// Returns the root cache key (currently always a constant magic string)
    fn root_key(&self) -> &str;
    /// Returns the global cache key (alias for root_key)
    fn global_cache_key(&self) -> &str;
    /// Returns the map of global file paths to their hashes
    fn global_file_hash_map(&self) -> &HashMap<RelativeUnixPathBuf, String>;
    /// Returns the hash of root external dependencies
    fn root_external_deps_hash(&self) -> &str;
    /// Returns the list of environment variable patterns
    fn env(&self) -> &[String];
    /// Returns the resolved environment variable map
    fn resolved_env_vars(&self) -> Option<&HashMap<String, String>>;
    /// Returns the list of pass-through environment variable patterns
    fn pass_through_env(&self) -> Option<&[String]>;
    /// Returns the environment mode (strict or loose)
    fn env_mode(&self) -> EnvMode;
    /// Returns whether framework inference is enabled
    fn framework_inference(&self) -> bool;
    /// Returns the list of dot-env files
    fn dot_env(&self) -> Option<&[RelativeUnixPathBuf]>;
}
