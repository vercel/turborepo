//! Task definition conversion from processed turbo.json.
//!
//! This module provides the `TaskDefinitionFromProcessed` trait for converting
//! `ProcessedTaskDefinition` to `TaskDefinition`.

use turbopath::RelativeUnixPath;
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_turbo_json::{
    ProcessedTaskDefinition, TOPOLOGICAL_PIPELINE_DELIMITER, TaskInputsFromProcessed,
    task_outputs_from_processed,
};
use turborepo_types::{TaskDefinition, TaskInputs};

use crate::BuilderError;

/// Extension trait for creating TaskDefinition from processed task definitions.
///
/// This allows for clean conversion from the turbo.json parsed representation
/// to the engine's internal task definition format.
pub trait TaskDefinitionFromProcessed {
    /// Creates a TaskDefinition from a ProcessedTaskDefinition
    fn from_processed(
        processed: ProcessedTaskDefinition,
        path_to_repo_root: &RelativeUnixPath,
    ) -> Result<TaskDefinition, BuilderError>;

    /// Helper method for tests that still use RawTaskDefinition.
    /// This is available in all builds to allow dependent crates' tests to use
    /// it.
    fn from_raw(
        raw_task: turborepo_turbo_json::RawTaskDefinition,
        path_to_repo_root: &RelativeUnixPath,
    ) -> Result<TaskDefinition, BuilderError>;
}

impl TaskDefinitionFromProcessed for TaskDefinition {
    fn from_processed(
        processed: ProcessedTaskDefinition,
        path_to_repo_root: &RelativeUnixPath,
    ) -> Result<TaskDefinition, BuilderError> {
        // Convert outputs with turbo_root resolution
        let outputs = processed
            .outputs
            .map(|outputs| task_outputs_from_processed(outputs, path_to_repo_root))
            .transpose()?
            .unwrap_or_default();

        let cache = processed.cache.is_none_or(|c| c.into_inner());
        let interactive = processed
            .interactive
            .as_ref()
            .map(|value| value.value)
            .unwrap_or_default();

        if let Some(interactive) = &processed.interactive {
            let (span, text) = interactive.span_and_text("turbo.json");
            if cache && interactive.value {
                return Err(BuilderError::TurboJson(
                    turborepo_turbo_json::Error::InteractiveNoCacheable { span, text },
                ));
            }
        }

        let persistent = *processed.persistent.unwrap_or_default();
        let interruptible = processed.interruptible.unwrap_or_default();
        if *interruptible && !persistent {
            let (span, text) = interruptible.span_and_text("turbo.json");
            return Err(BuilderError::TurboJson(
                turborepo_turbo_json::Error::InterruptibleButNotPersistent { span, text },
            ));
        }

        let mut topological_dependencies: Vec<Spanned<TaskName>> = Vec::new();
        let mut task_dependencies: Vec<Spanned<TaskName>> = Vec::new();
        if let Some(depends_on) = processed.depends_on {
            for dependency in depends_on.deps {
                let (dependency, depspan) = dependency.split();
                let dependency: String = dependency.into();
                if let Some(topo_dependency) =
                    dependency.strip_prefix(TOPOLOGICAL_PIPELINE_DELIMITER)
                {
                    topological_dependencies.push(depspan.to(topo_dependency.to_string().into()));
                } else {
                    task_dependencies.push(depspan.to(dependency.into()));
                }
            }
        }

        task_dependencies.sort_by(|a, b| a.value.cmp(&b.value));
        topological_dependencies.sort_by(|a, b| a.value.cmp(&b.value));

        let env = processed.env.map(|env| env.vars).unwrap_or_default();

        // Convert inputs with turbo_root resolution
        let inputs = processed
            .inputs
            .map(|inputs| TaskInputs::from_processed(inputs, path_to_repo_root))
            .unwrap_or_default();

        let pass_through_env = processed.pass_through_env.map(|env| env.vars);

        let with = processed.with.map(|with_tasks| with_tasks.tasks);

        Ok(TaskDefinition {
            outputs,
            cache,
            topological_dependencies,
            task_dependencies,
            env,
            inputs,
            pass_through_env,
            output_logs: *processed.output_logs.unwrap_or_default(),
            persistent,
            interruptible: *interruptible,
            interactive,
            env_mode: processed.env_mode.map(|mode| *mode.as_inner()),
            with,
        })
    }

    fn from_raw(
        raw_task: turborepo_turbo_json::RawTaskDefinition,
        path_to_repo_root: &RelativeUnixPath,
    ) -> Result<TaskDefinition, BuilderError> {
        use turborepo_turbo_json::FutureFlags;
        // Use default FutureFlags for backward compatibility
        let processed = ProcessedTaskDefinition::from_raw(raw_task, &FutureFlags::default())?;
        <TaskDefinition as TaskDefinitionFromProcessed>::from_processed(
            processed,
            path_to_repo_root,
        )
    }
}
