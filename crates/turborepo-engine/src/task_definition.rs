//! Task definition conversion from processed turbo.json.
//!
//! This module provides the `TaskDefinitionFromProcessed` trait for converting
//! `ProcessedTaskDefinition` to `TaskDefinition`.

use turbopath::RelativeUnixPath;
use turborepo_errors::Spanned;
use turborepo_task_id::TaskName;
use turborepo_turbo_json::{
    ProcessedTaskDefinition, TOPOLOGICAL_PIPELINE_DELIMITER, TaskInputsFromProcessed,
    incremental_partitions_from_processed, task_outputs_from_processed,
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

        let incremental = processed
            .incremental
            .map(|partitions| incremental_partitions_from_processed(partitions, path_to_repo_root))
            .transpose()
            .map_err(BuilderError::TurboJson)?;

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
            incremental,
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

/// Prepends global input globs to a task's `TaskInputs`.
///
/// When `futureFlags.globalConfiguration` is enabled, global input files
/// are treated as implicit inputs for every task instead of being folded
/// into the global hash. This lets tasks exclude specific global files
/// via negation globs (e.g. `!$TURBO_ROOT$/config.txt`).
///
/// If the task had no explicit `inputs` key (i.e. it was using the
/// default "hash everything in the package" behavior), `default` is set
/// to `true` so that package files are still included alongside the
/// global inputs.
pub fn prepend_global_inputs(
    inputs: &mut TaskInputs,
    had_explicit_inputs: bool,
    global_deps: &[String],
    path_to_repo_root: &RelativeUnixPath,
) {
    if global_deps.is_empty() {
        return;
    }

    if !had_explicit_inputs {
        inputs.default = true;
    }

    let mut global_globs: Vec<String> = global_deps
        .iter()
        .map(|dep| {
            if let Some(exclusion) = dep.strip_prefix('!') {
                format!("!{path_to_repo_root}/{exclusion}")
            } else {
                format!("{path_to_repo_root}/{dep}")
            }
        })
        .collect();
    global_globs.append(&mut inputs.globs);
    inputs.globs = global_globs;
}

#[cfg(test)]
mod tests {
    use turbopath::RelativeUnixPathBuf;
    use turborepo_types::TaskInputs;

    use super::*;

    #[test]
    fn test_prepend_global_inputs_basic() {
        let path_to_root = RelativeUnixPathBuf::new("../..").expect("valid path");
        let mut inputs = TaskInputs {
            globs: vec!["src/**".to_string()],
            default: false,
        };

        prepend_global_inputs(
            &mut inputs,
            true,
            &["config.txt".to_string()],
            &path_to_root,
        );

        assert_eq!(
            inputs.globs,
            vec!["../../config.txt", "src/**"],
            "global dep should be prepended with root-relative path"
        );
        assert!(
            !inputs.default,
            "default should remain false when task had explicit inputs"
        );
    }

    #[test]
    fn test_prepend_global_inputs_sets_default_when_no_explicit_inputs() {
        let path_to_root = RelativeUnixPathBuf::new("../..").expect("valid path");
        let mut inputs = TaskInputs::default();

        prepend_global_inputs(
            &mut inputs,
            false,
            &["config.txt".to_string()],
            &path_to_root,
        );

        assert_eq!(inputs.globs, vec!["../../config.txt"]);
        assert!(
            inputs.default,
            "default should be set to true so package files are still hashed"
        );
    }

    #[test]
    fn test_prepend_global_inputs_handles_negation() {
        let path_to_root = RelativeUnixPathBuf::new("..").expect("valid path");
        let mut inputs = TaskInputs {
            globs: vec!["**".to_string()],
            default: false,
        };

        prepend_global_inputs(
            &mut inputs,
            true,
            &["config/**".to_string(), "!config/local.txt".to_string()],
            &path_to_root,
        );

        assert_eq!(
            inputs.globs,
            vec!["../config/**", "!../config/local.txt", "**"],
        );
    }

    #[test]
    fn test_prepend_global_inputs_noop_when_empty() {
        let path_to_root = RelativeUnixPathBuf::new("../..").expect("valid path");
        let mut inputs = TaskInputs {
            globs: vec!["src/**".to_string()],
            default: false,
        };
        let original = inputs.clone();

        prepend_global_inputs(&mut inputs, true, &[], &path_to_root);

        assert_eq!(inputs.globs, original.globs);
        assert_eq!(inputs.default, original.default);
    }
}
