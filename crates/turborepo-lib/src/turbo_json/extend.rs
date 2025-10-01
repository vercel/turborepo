//! Module for code related to "extends" behavior for task definitions

use super::processed::{
    ProcessedDependsOn, ProcessedEnv, ProcessedInputs, ProcessedOutputs, ProcessedPassThroughEnv,
    ProcessedTaskDefinition, ProcessedWith,
};

/// Trait for types that can be merged with extends behavior
trait Extendable {
    /// Merges another instance into self.
    /// If the other instance has `extends: true`, it extends the current value.
    /// Otherwise, it replaces the current value.
    fn extend(&mut self, other: Self);
}

/// Macro to handle the extend/replace logic for a field
macro_rules! merge_field_vec {
    ($self:ident, $other:ident, $field:ident) => {
        if $other.extends {
            $self.$field.extend($other.$field);
        } else {
            $self.$field = $other.$field;
        }
    };
}

impl Extendable for ProcessedDependsOn {
    fn extend(&mut self, other: Self) {
        merge_field_vec!(self, other, deps);
        self.extends = other.extends;
    }
}

impl Extendable for ProcessedEnv {
    fn extend(&mut self, other: Self) {
        merge_field_vec!(self, other, vars);
        // Sort and dedup for env vars
        if other.extends {
            self.vars.sort();
            self.vars.dedup();
        }
        self.extends = other.extends;
    }
}

impl Extendable for ProcessedOutputs {
    fn extend(&mut self, other: Self) {
        merge_field_vec!(self, other, globs);
        self.extends = other.extends;
    }
}

impl Extendable for ProcessedPassThroughEnv {
    fn extend(&mut self, other: Self) {
        merge_field_vec!(self, other, vars);
        // Sort and dedup for env vars
        if other.extends {
            self.vars.sort();
            self.vars.dedup();
        }
        self.extends = other.extends;
    }
}

impl Extendable for ProcessedWith {
    fn extend(&mut self, other: Self) {
        merge_field_vec!(self, other, tasks);
        self.extends = other.extends;
    }
}

impl Extendable for ProcessedInputs {
    fn extend(&mut self, other: Self) {
        merge_field_vec!(self, other, globs);
        // Handle the default flag specially
        if other.extends {
            // When extending, OR the default flags
            self.default = self.default || other.default;
        } else {
            // When replacing, use the other's default
            self.default = other.default;
        }
        self.extends = other.extends;
    }
}

impl FromIterator<ProcessedTaskDefinition> for ProcessedTaskDefinition {
    fn from_iter<T: IntoIterator<Item = ProcessedTaskDefinition>>(iter: T) -> Self {
        iter.into_iter()
            .fold(ProcessedTaskDefinition::default(), |mut def, other| {
                def.merge(other);
                def
            })
    }
}

macro_rules! set_field {
    ($this:ident, $other:ident, $field:ident) => {{
        if let Some(field) = $other.$field {
            $this.$field = field.into();
        }
    }};
}

macro_rules! merge_field {
    ($this:ident, $other:ident, $field:ident) => {{
        if let Some(other_field) = $other.$field {
            match &mut $this.$field {
                Some(self_field) => {
                    // Merge using the Mergeable trait
                    self_field.extend(other_field);
                }
                None => {
                    // No existing value, just set it
                    $this.$field = Some(other_field);
                }
            }
        }
    }};
}

impl ProcessedTaskDefinition {
    // Merges another ProcessedTaskDefinition into this one
    // Array fields use the Mergeable trait to handle extends behavior
    pub fn merge(&mut self, other: ProcessedTaskDefinition) {
        // Array fields that support extends behavior
        merge_field!(self, other, outputs);
        merge_field!(self, other, depends_on);
        merge_field!(self, other, inputs);
        merge_field!(self, other, env);
        merge_field!(self, other, pass_through_env);
        merge_field!(self, other, with);

        // Non-array fields that are simply replaced
        let other_has_range = other.cache.as_ref().is_some_and(|c| c.range.is_some());
        let self_does_not_have_range = self.cache.as_ref().is_some_and(|c| c.range.is_none());

        if other.cache.is_some()
            // If other has range info and we're missing it, carry it over
            || (other_has_range && self_does_not_have_range)
        {
            self.cache = other.cache;
        }
        set_field!(self, other, output_logs);
        set_field!(self, other, persistent);
        set_field!(self, other, interruptible);
        set_field!(self, other, interactive);
        set_field!(self, other, env_mode);
    }
}

#[cfg(test)]
mod test {
    use turborepo_errors::Spanned;
    use turborepo_unescape::UnescapedString;

    use super::*;
    use crate::{
        cli::OutputLogsMode,
        turbo_json::{
            processed::{ProcessedEnv, ProcessedInputs, ProcessedOutputs},
            FutureFlags,
        },
    };

    // Shared test fixtures
    fn create_base_task() -> ProcessedTaskDefinition {
        ProcessedTaskDefinition {
            cache: Some(Spanned::new(true)),
            persistent: Some(Spanned::new(false)),
            outputs: Some(
                ProcessedOutputs::new(
                    vec![Spanned::new(UnescapedString::from("dist/**"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            inputs: Some(
                ProcessedInputs::new(
                    vec![Spanned::new(UnescapedString::from("src/**"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            env: Some(
                ProcessedEnv::new(
                    vec![Spanned::new(UnescapedString::from("NODE_ENV"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            depends_on: None,
            pass_through_env: None,
            output_logs: None,
            interruptible: None,
            interactive: None,
            env_mode: None,
            with: None,
        }
    }

    fn create_override_task() -> ProcessedTaskDefinition {
        ProcessedTaskDefinition {
            cache: Some(Spanned::new(false)),
            persistent: Some(Spanned::new(true)),
            outputs: Some(
                ProcessedOutputs::new(
                    vec![Spanned::new(UnescapedString::from("build/**"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            inputs: Some(
                ProcessedInputs::new(
                    vec![Spanned::new(UnescapedString::from("lib/**"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            env: Some(
                ProcessedEnv::new(
                    vec![Spanned::new(UnescapedString::from("PROD_ENV"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            output_logs: Some(Spanned::new(OutputLogsMode::Full)),
            interruptible: Some(Spanned::new(true)),
            depends_on: None,
            pass_through_env: None,
            interactive: None,
            env_mode: None,
            with: None,
        }
    }

    fn create_partial_task() -> ProcessedTaskDefinition {
        ProcessedTaskDefinition {
            persistent: Some(Spanned::new(true)),
            output_logs: Some(Spanned::new(OutputLogsMode::HashOnly)),
            cache: None,
            outputs: None,
            inputs: None,
            env: None,
            depends_on: None,
            pass_through_env: None,
            interruptible: None,
            interactive: None,
            env_mode: None,
            with: None,
        }
    }

    #[test]
    fn test_other_takes_priority() {
        let mut base = create_base_task();
        let override_task = create_override_task();

        // Store original values for comparison
        let original_cache = base.cache.clone();
        let original_persistent = base.persistent.clone();
        let original_outputs = base.outputs.clone();

        // Perform merge
        base.merge(override_task.clone());

        // All fields from override_task should take priority
        assert_eq!(base.cache, override_task.cache);
        assert_eq!(base.persistent, override_task.persistent);
        assert_eq!(base.outputs, override_task.outputs);
        assert_eq!(base.inputs, override_task.inputs);
        assert_eq!(base.env, override_task.env);
        assert_eq!(base.output_logs, override_task.output_logs);
        assert_eq!(base.interruptible, override_task.interruptible);

        // Verify values actually changed
        assert_ne!(base.cache, original_cache);
        assert_ne!(base.persistent, original_persistent);
        assert_ne!(base.outputs, original_outputs);
    }

    #[test]
    fn test_partial_merge_preserves_existing() {
        let mut base = create_base_task();
        let partial = create_partial_task();

        // Store original values that should be preserved
        let original_cache = base.cache.clone();
        let original_outputs = base.outputs.clone();
        let original_inputs = base.inputs.clone();
        let original_env = base.env.clone();

        // Perform merge
        base.merge(partial.clone());

        // Fields present in partial should be overridden
        assert_eq!(base.persistent, partial.persistent);
        assert_eq!(base.output_logs, partial.output_logs);

        // Fields not present in partial should be preserved
        assert_eq!(base.cache, original_cache);
        assert_eq!(base.outputs, original_outputs);
        assert_eq!(base.inputs, original_inputs);
        assert_eq!(base.env, original_env);

        // Fields not set in either should remain None
        assert_eq!(base.interruptible, None);
        assert_eq!(base.interactive, None);
    }

    #[test]
    fn test_from_iter_last_takes_priority() {
        let first = create_base_task();
        let second = create_partial_task();
        let third = create_override_task();

        let tasks = vec![first.clone(), second.clone(), third.clone()];
        let result: ProcessedTaskDefinition = tasks.into_iter().collect();

        // Fields present in the last task (third) should take priority
        assert_eq!(result.cache, third.cache);
        assert_eq!(result.persistent, third.persistent);
        assert_eq!(result.outputs, third.outputs);
        assert_eq!(result.inputs, third.inputs);
        assert_eq!(result.env, third.env);
        assert_eq!(result.output_logs, third.output_logs);
        assert_eq!(result.interruptible, third.interruptible);

        // Fields only present in earlier tasks should be preserved if not
        // overridden (none in this case since third task overrides all
        // fields that first task had)
    }

    #[test]
    fn test_from_iter_combines_across_multiple_tasks() {
        let first = ProcessedTaskDefinition {
            cache: Some(Spanned::new(true)),
            outputs: Some(
                ProcessedOutputs::new(
                    vec![Spanned::new(UnescapedString::from("dist/**"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            ..Default::default()
        };

        let second = ProcessedTaskDefinition {
            persistent: Some(Spanned::new(false)),
            inputs: Some(
                ProcessedInputs::new(
                    vec![Spanned::new(UnescapedString::from("src/**"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            ..Default::default()
        };

        let third = ProcessedTaskDefinition {
            env: Some(
                ProcessedEnv::new(
                    vec![Spanned::new(UnescapedString::from("NODE_ENV"))],
                    &FutureFlags::default(),
                )
                .unwrap(),
            ),
            output_logs: Some(Spanned::new(OutputLogsMode::Full)),
            // Override cache from first task
            cache: Some(Spanned::new(false)),
            ..Default::default()
        };

        let tasks = vec![first.clone(), second.clone(), third.clone()];
        let result: ProcessedTaskDefinition = tasks.into_iter().collect();

        // Last task's cache should override first task's cache
        assert_eq!(result.cache, third.cache);

        // Fields from second task should be preserved since not overridden
        assert_eq!(result.persistent, second.persistent);
        assert_eq!(result.inputs, second.inputs);

        // Fields from first task should be preserved since not overridden later
        assert_eq!(result.outputs, first.outputs);

        // Fields from third task
        assert_eq!(result.env, third.env);
        assert_eq!(result.output_logs, third.output_logs);
    }

    #[test]
    fn test_from_iter_empty_iterator() {
        let empty_vec: Vec<ProcessedTaskDefinition> = vec![];
        let result: ProcessedTaskDefinition = empty_vec.into_iter().collect();

        // Should be equivalent to default
        assert_eq!(result, ProcessedTaskDefinition::default());
    }

    #[test]
    fn test_from_iter_single_task() {
        let single_task = create_base_task();
        let tasks = vec![single_task.clone()];
        let result: ProcessedTaskDefinition = tasks.into_iter().collect();

        assert_eq!(result, single_task);
    }

    // Reusable fixtures for array fields
    fn env_base() -> ProcessedEnv {
        ProcessedEnv {
            vars: vec!["BASE_ENV".to_string()],
            extends: false,
        }
    }

    fn env_override() -> ProcessedEnv {
        ProcessedEnv {
            vars: vec!["OVERRIDE_ENV".to_string()],
            extends: false,
        }
    }

    fn env_extending() -> ProcessedEnv {
        ProcessedEnv {
            vars: vec!["OVERRIDE_ENV".to_string()],
            extends: true,
        }
    }

    fn deps_base() -> ProcessedDependsOn {
        ProcessedDependsOn {
            deps: vec![Spanned::new(UnescapedString::from("build"))],
            extends: false,
        }
    }

    fn deps_test() -> ProcessedDependsOn {
        ProcessedDependsOn {
            deps: vec![Spanned::new(UnescapedString::from("test"))],
            extends: false,
        }
    }

    fn deps_extending() -> ProcessedDependsOn {
        ProcessedDependsOn {
            deps: vec![Spanned::new(UnescapedString::from("test"))],
            extends: true,
        }
    }

    fn with_task1() -> ProcessedWith {
        use turborepo_task_id::TaskName;
        ProcessedWith {
            tasks: vec![Spanned::new(TaskName::from("task1"))],
            extends: false,
        }
    }

    fn with_task2_extending() -> ProcessedWith {
        use turborepo_task_id::TaskName;
        ProcessedWith {
            tasks: vec![Spanned::new(TaskName::from("task2"))],
            extends: true,
        }
    }

    fn with_task3() -> ProcessedWith {
        use turborepo_task_id::TaskName;
        ProcessedWith {
            tasks: vec![Spanned::new(TaskName::from("task3"))],
            extends: false,
        }
    }

    fn inputs_base() -> ProcessedInputs {
        ProcessedInputs {
            globs: vec![],
            default: false,
            extends: false,
        }
    }

    fn inputs_extending_with_default() -> ProcessedInputs {
        ProcessedInputs {
            globs: vec![],
            default: true,
            extends: true,
        }
    }

    fn outputs_base() -> ProcessedOutputs {
        ProcessedOutputs {
            globs: vec![],
            extends: false,
        }
    }

    fn outputs_extending() -> ProcessedOutputs {
        ProcessedOutputs {
            globs: vec![],
            extends: true,
        }
    }

    #[test]
    fn test_merge_with_extends_true() {
        let mut base = ProcessedTaskDefinition {
            inputs: Some(inputs_base()),
            outputs: Some(outputs_base()),
            env: Some(env_base()),
            depends_on: Some(deps_base()),
            with: Some(with_task1()),
            ..Default::default()
        };

        let extending = ProcessedTaskDefinition {
            inputs: Some(inputs_extending_with_default()),
            outputs: Some(outputs_extending()),
            env: Some(env_extending()),
            depends_on: Some(deps_extending()),
            with: Some(with_task2_extending()),
            ..Default::default()
        };

        base.merge(extending);

        // Verify extends behavior
        assert!(base.inputs.as_ref().unwrap().default); // OR'd
        assert!(base.inputs.as_ref().unwrap().extends);
        assert_eq!(
            base.env.as_ref().unwrap().vars,
            vec!["BASE_ENV".to_string(), "OVERRIDE_ENV".to_string()]
        );
        assert_eq!(base.depends_on.as_ref().unwrap().deps.len(), 2);
        assert_eq!(base.with.as_ref().unwrap().tasks.len(), 2);
    }

    #[test]
    fn test_merge_with_extends_false() {
        let mut base = ProcessedTaskDefinition {
            env: Some(env_base()),
            depends_on: Some(deps_base()),
            with: Some(with_task1()),
            ..Default::default()
        };

        let replacing = ProcessedTaskDefinition {
            env: Some(env_override()),
            depends_on: Some(deps_test()),
            with: Some(with_task3()),
            ..Default::default()
        };

        base.merge(replacing);

        // Verify replace behavior
        assert_eq!(base.env, Some(env_override()));
        assert_eq!(base.depends_on, Some(deps_test()));
        assert_eq!(base.with, Some(with_task3()));
    }

    #[test]
    fn test_merge_chain_with_extends_then_replace() {
        // Test that when chaining: base -> extending -> replacing
        // The final replacing task overrides everything, not extends

        let base = ProcessedTaskDefinition {
            depends_on: Some(deps_base()), // has "build"
            env: Some(env_base()),         // has "BASE_ENV"
            ..Default::default()
        };

        // Middle task extends the base
        let extending = ProcessedTaskDefinition {
            depends_on: Some(deps_extending()), // has "test" with extends: true
            env: Some(env_extending()),         // has "OVERRIDE_ENV" with extends: true
            ..Default::default()
        };

        // Final task replaces (no extends)
        let replacing = ProcessedTaskDefinition {
            depends_on: Some(ProcessedDependsOn {
                deps: vec![Spanned::new(UnescapedString::from("lint"))],
                extends: false, // This should replace, not extend
            }),
            env: Some(ProcessedEnv {
                vars: vec!["FINAL_ENV".to_string()],
                extends: false, // This should replace, not extend
            }),
            ..Default::default()
        };

        // Apply the chain of merges
        let result = ProcessedTaskDefinition::from_iter(vec![base, extending, replacing]);

        assert_eq!(
            result.depends_on.as_ref().unwrap().deps,
            vec![Spanned::new(UnescapedString::from("lint"))]
        );

        // Verify the extends flags are now false
        assert!(!result.depends_on.as_ref().unwrap().extends);
    }
}
