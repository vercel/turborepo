//! Run summary tracking and reporting for Turborepo.
//!
//! This crate provides types and traits for tracking task execution
//! and generating run summaries.

mod duration;
mod execution;
mod global_hash;
pub mod observability;
mod scm;
mod task;
mod task_factory;
mod tracker;

pub use duration::TurboDuration;
pub use execution::{
    ExecutionSummary, ExecutionTracker, IncrementalCacheSummary, SummaryState, TaskState,
    TaskSummaryInfo, TaskTracker,
};
pub use global_hash::{GlobalEnvConfiguration, GlobalEnvVarSummary, GlobalHashSummary};
pub use observability::Handle as ObservabilityHandle;
pub use scm::SCMState;
pub use task::{
    SharedTaskSummary, SinglePackageTaskSummary, TaskCacheSummary, TaskEnvConfiguration,
    TaskEnvVarSummary, TaskExecutionSummary, TaskSummary, TaskSummaryTaskDefinition,
};
pub use task_factory::{Error as TaskFactoryError, TaskSummaryFactory, get_external_deps_hash};
pub use tracker::{Error, RunSummary, RunTracker, SinglePackageRunSummary};
