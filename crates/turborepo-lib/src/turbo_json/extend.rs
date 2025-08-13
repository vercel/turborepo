//! Module for code related to "extends" behavior for task definitions

use super::processed::ProcessedTaskDefinition;

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

impl ProcessedTaskDefinition {
    // Merges another ProcessedTaskDefinition into this one
    // By default any fields present on `other` will override present fields.
    pub fn merge(&mut self, other: ProcessedTaskDefinition) {
        set_field!(self, other, outputs);

        let other_has_range = other.cache.as_ref().is_some_and(|c| c.range.is_some());
        let self_does_not_have_range = self.cache.as_ref().is_some_and(|c| c.range.is_none());

        if other.cache.is_some()
            // If other has range info and we're missing it, carry it over
            || (other_has_range && self_does_not_have_range)
        {
            self.cache = other.cache;
        }
        set_field!(self, other, depends_on);
        set_field!(self, other, inputs);
        set_field!(self, other, output_logs);
        set_field!(self, other, persistent);
        set_field!(self, other, interruptible);
        set_field!(self, other, env);
        set_field!(self, other, pass_through_env);
        set_field!(self, other, interactive);
        set_field!(self, other, env_mode);
        set_field!(self, other, with);
    }
}

#[cfg(test)]
mod test {
    use turborepo_errors::Spanned;
    use turborepo_unescape::UnescapedString;

    use super::*;
    use crate::{
        cli::OutputLogsMode,
        turbo_json::processed::{ProcessedEnv, ProcessedInputs, ProcessedOutputs},
    };

    // Shared test fixtures
    fn create_base_task() -> ProcessedTaskDefinition {
        ProcessedTaskDefinition {
            cache: Some(Spanned::new(true)),
            persistent: Some(Spanned::new(false)),
            outputs: Some(
                ProcessedOutputs::new(vec![Spanned::new(UnescapedString::from("dist/**"))])
                    .unwrap(),
            ),
            inputs: Some(
                ProcessedInputs::new(vec![Spanned::new(UnescapedString::from("src/**"))]).unwrap(),
            ),
            env: Some(ProcessedEnv(vec![Spanned::new(UnescapedString::from(
                "NODE_ENV",
            ))])),
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
                ProcessedOutputs::new(vec![Spanned::new(UnescapedString::from("build/**"))])
                    .unwrap(),
            ),
            inputs: Some(
                ProcessedInputs::new(vec![Spanned::new(UnescapedString::from("lib/**"))]).unwrap(),
            ),
            env: Some(ProcessedEnv(vec![Spanned::new(UnescapedString::from(
                "PROD_ENV",
            ))])),
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
                ProcessedOutputs::new(vec![Spanned::new(UnescapedString::from("dist/**"))])
                    .unwrap(),
            ),
            ..Default::default()
        };

        let second = ProcessedTaskDefinition {
            persistent: Some(Spanned::new(false)),
            inputs: Some(
                ProcessedInputs::new(vec![Spanned::new(UnescapedString::from("src/**"))]).unwrap(),
            ),
            ..Default::default()
        };

        let third = ProcessedTaskDefinition {
            env: Some(ProcessedEnv(vec![Spanned::new(UnescapedString::from(
                "NODE_ENV",
            ))])),
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
}
