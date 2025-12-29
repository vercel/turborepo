//! Task hashing module - delegates to turborepo-task-hash crate.
//!
//! This module re-exports types from turborepo-task-hash and provides
//! trait implementations for turborepo-lib types.

use turborepo_task_id::TaskId;
use turborepo_types::{TaskInputs, TaskOutputs};

// Re-export all public types from turborepo-task-hash
pub use turborepo_task_hash::{
    get_external_deps_hash, get_internal_deps_hash, Error, PackageInputsHashes, TaskHashTracker,
    TaskHashTrackerState,
};

use crate::{opts::RunOpts, task_graph::TaskDefinition};

// Implement TaskDefinitionHashInfo for TaskDefinition
impl turborepo_task_hash::TaskDefinitionHashInfo for TaskDefinition {
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
        TaskDefinition::hashable_outputs(self, task_id)
    }
}

// Implement RunOptsHashInfo for RunOpts
impl turborepo_task_hash::RunOptsHashInfo for RunOpts {
    fn framework_inference(&self) -> bool {
        self.framework_inference
    }

    fn single_package(&self) -> bool {
        self.single_package
    }

    fn pass_through_args(&self) -> &[String] {
        &self.pass_through_args
    }
}

/// Type alias for TaskHasher specialized with RunOpts
pub type TaskHasher<'a> = turborepo_task_hash::TaskHasher<'a, RunOpts>;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_hash_tracker_is_send_and_sync() {
        // We need the tracker to implement these traits as multiple tasks will query
        // and write to it
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<TaskHashTracker>();
        assert_sync::<TaskHashTracker>();
    }
}
