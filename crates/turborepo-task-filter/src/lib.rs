//! Task-level filtering and affected detection for Turborepo.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod task_change_detector;
mod task_filter;

pub use task_change_detector::*;
pub use task_filter::*;
#[cfg(test)]
pub use turborepo_engine::Building;
use turborepo_engine::Built;
pub use turborepo_engine::TaskNode;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_task_id::TaskId;
use turborepo_types::{TaskCommandOverride, TaskDefinition};

/// Engine specialized with Turborepo task definitions.
pub type Engine<S = Built> = turborepo_engine::Engine<S, TaskDefinition>;

/// Error returned while resolving task filters.
pub type Error = turborepo_scm::Error;

fn task_has_command(engine: &Engine, package_graph: &PackageGraph, task: &TaskId<'static>) -> bool {
    if task.task() == "proxy" {
        return true;
    }

    let Some(context) = package_graph.package_task_context(&PackageName::from(task.package()))
    else {
        return false;
    };

    match engine
        .task_definition(task)
        .and_then(|definition| definition.command.as_ref())
    {
        Some(TaskCommandOverride::Argv(_)) => true,
        Some(TaskCommandOverride::OptOut) => false,
        None => context.native_tasks().defines(task.task()),
    }
}
