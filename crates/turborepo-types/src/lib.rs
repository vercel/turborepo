//! Shared types for Turborepo
//!
//! This crate contains types that are used across multiple crates in the
//! turborepo ecosystem. It serves as a foundation layer to avoid circular
//! dependencies between higher-level crates.

use std::fmt;

use biome_deserialize_macros::Deserializable;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

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
}
