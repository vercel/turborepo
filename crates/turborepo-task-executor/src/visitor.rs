//! Visitor support types and traits.
//!
//! This module provides shared types and trait definitions for the task graph
//! visitor. The concrete `Visitor` implementation remains in `turborepo-lib`,
//! but these abstractions allow for decoupling and testing.

use std::sync::OnceLock;

use regex::Regex;
use tokio::sync::mpsc;
use turborepo_repository::package_graph::PackageInfo;
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::task::PackageTaskEventBuilder;
use turborepo_types::{EnvMode, StopExecution, TaskDefinition};

use crate::HashTrackerProvider;

/// Trait for providing task execution messages from the engine.
pub trait EngineProvider: Send + Sync {
    /// The error type for engine execution.
    type Error: std::error::Error + Send;

    /// Returns an iterator over all task IDs in the engine.
    fn tasks(&self) -> Box<dyn Iterator<Item = TaskId<'static>> + '_>;

    /// Returns the task definition for a given task ID.
    fn task_definition(&self, task_id: &TaskId) -> Option<&TaskDefinition>;

    /// Returns the dependencies for a given task ID.
    fn dependencies(&self, task_id: &TaskId) -> Option<Vec<TaskId<'static>>>;
}

/// Trait for providing task hashing functionality.
pub trait TaskHashProvider: Send {
    /// The error type for hash calculation.
    type Error: std::error::Error + Send;

    /// The hash tracker type returned by this provider.
    type HashTracker: HashTrackerProvider;

    /// Calculate the hash for a task.
    fn calculate_task_hash(
        &self,
        task_id: &TaskId,
        task_definition: &TaskDefinition,
        env_mode: EnvMode,
        workspace_info: &PackageInfo,
        dependency_set: Vec<TaskId<'static>>,
        telemetry: PackageTaskEventBuilder,
    ) -> Result<String, Self::Error>;

    /// Get the hash tracker.
    fn task_hash_tracker(&self) -> Self::HashTracker;
}

/// Callback for task completion.
pub type TaskCallback = tokio::sync::oneshot::Sender<Result<(), StopExecution>>;

/// Message from the engine containing task info and completion callback.
pub struct EngineMessage {
    pub task_id: TaskId<'static>,
    pub callback: TaskCallback,
}

/// Trait for engine execution.
pub trait EngineExecutor: Send {
    /// The error type for execution.
    type Error: std::error::Error + Send + 'static;

    /// Execute the engine and send task messages to the provided sender.
    fn execute(
        self,
        concurrency: usize,
        sender: mpsc::Sender<EngineMessage>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

/// Check if a command invokes turbo (which would create a recursive loop).
pub fn turbo_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)turbo(?:$|\s)").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turbo_regex() {
        let re = turbo_regex();
        assert!(re.is_match("turbo"));
        assert!(re.is_match("turbo build"));
        assert!(re.is_match("npx turbo build"));
        assert!(!re.is_match("turbopack"));
        assert!(!re.is_match("myturbo"));
    }
}
