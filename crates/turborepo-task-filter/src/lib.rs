//! Task-level filtering and affected detection for Turborepo.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod task_change_detector;
mod task_filter;

pub use task_change_detector::*;
pub use task_filter::*;
#[cfg(test)]
pub use turborepo_engine::Building;
pub use turborepo_engine::TaskNode;
use turborepo_engine::{Built, task_has_command};
use turborepo_types::TaskDefinition;

/// Engine specialized with Turborepo task definitions.
pub type Engine<S = Built> = turborepo_engine::Engine<S, TaskDefinition>;

/// Error returned while resolving task filters.
pub type Error = turborepo_scm::Error;
