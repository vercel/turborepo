//! Task hashing module - delegates to turborepo-task-hash crate.
//!
//! This module provides trait implementations for turborepo-lib types.

use crate::opts::RunOpts;

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
    use turborepo_task_hash::TaskHashTracker;

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
