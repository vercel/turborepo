//! Task hashing module - delegates to turborepo-task-hash crate.
//!
//! This module provides task hashing types specialized for turborepo-lib.

use turborepo_run_opts::RunOpts;

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
