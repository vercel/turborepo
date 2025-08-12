//! Processed task definition types with DSL token handling

use turborepo_errors::Spanned;
use turborepo_unescape::UnescapedString;

use super::RawTaskDefinition;
use crate::cli::{EnvMode, OutputLogsMode};

/// Processed depends_on field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedDependsOn(pub Spanned<Vec<Spanned<UnescapedString>>>);

/// Processed env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedEnv(pub Vec<Spanned<UnescapedString>>);

/// Processed inputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedInputs(pub Vec<Spanned<UnescapedString>>);

/// Processed pass_through_env field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedPassThroughEnv(pub Vec<Spanned<UnescapedString>>);

/// Processed outputs field with DSL detection
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedOutputs(pub Vec<Spanned<UnescapedString>>);

/// Intermediate representation for task definitions with DSL processing
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessedTaskDefinition {
    pub cache: Option<Spanned<bool>>,
    pub depends_on: Option<ProcessedDependsOn>,
    pub env: Option<ProcessedEnv>,
    pub inputs: Option<ProcessedInputs>,
    pub pass_through_env: Option<ProcessedPassThroughEnv>,
    pub persistent: Option<Spanned<bool>>,
    pub interruptible: Option<Spanned<bool>>,
    pub outputs: Option<ProcessedOutputs>,
    pub output_logs: Option<Spanned<OutputLogsMode>>,
    pub interactive: Option<Spanned<bool>>,
    pub env_mode: Option<Spanned<EnvMode>>,
    pub with: Option<Vec<Spanned<UnescapedString>>>,
}

impl ProcessedTaskDefinition {
    /// Creates a processed task definition from raw task
    pub fn from_raw(raw_task: RawTaskDefinition) -> Self {
        ProcessedTaskDefinition {
            cache: raw_task.cache,
            depends_on: raw_task.depends_on.map(ProcessedDependsOn),
            env: raw_task.env.map(ProcessedEnv),
            inputs: raw_task.inputs.map(ProcessedInputs),
            pass_through_env: raw_task.pass_through_env.map(ProcessedPassThroughEnv),
            persistent: raw_task.persistent,
            interruptible: raw_task.interruptible,
            outputs: raw_task.outputs.map(ProcessedOutputs),
            output_logs: raw_task.output_logs,
            interactive: raw_task.interactive,
            env_mode: raw_task.env_mode,
            with: raw_task.with,
        }
    }

    /// Converts back to RawTaskDefinition
    pub fn into_raw(self) -> RawTaskDefinition {
        RawTaskDefinition {
            cache: self.cache,
            depends_on: self.depends_on.map(|d| d.0),
            env: self.env.map(|e| e.0),
            inputs: self.inputs.map(|i| i.0),
            pass_through_env: self.pass_through_env.map(|p| p.0),
            persistent: self.persistent,
            interruptible: self.interruptible,
            outputs: self.outputs.map(|o| o.0),
            output_logs: self.output_logs,
            interactive: self.interactive,
            env_mode: self.env_mode,
            with: self.with,
        }
    }
}
