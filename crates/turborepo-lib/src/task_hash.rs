//! Task hashing module - delegates to turborepo-task-hash crate.
//!
//! This module re-exports types from turborepo-task-hash and provides
//! trait implementations for turborepo-lib types.

// Re-export all public types from turborepo-task-hash
pub use turborepo_task_hash::{
    get_external_deps_hash, get_global_hash_inputs, get_internal_deps_hash, global_hash, Error,
    GlobalHashableInputs, PackageInputsHashes, TaskHashTracker, TaskHashTrackerState,
};

use crate::opts::RunOpts;

// Note: TaskDefinitionHashInfo is now implemented for TaskDefinition
// directly in turborepo-task-hash crate.

// Implement RunOptsHashInfo for RunOpts
impl turborepo_types::RunOptsHashInfo for RunOpts {
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
