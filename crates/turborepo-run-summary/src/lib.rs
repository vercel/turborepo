//! Run summary tracking and reporting for Turborepo.
//!
//! This crate provides types and traits for tracking task execution
//! and generating run summaries.

mod duration;
mod execution;
mod global_hash;
mod scm;
mod task;
mod task_factory;
mod tracker;

pub use duration::TurboDuration;
pub use execution::{
    ExecutionSummary, ExecutionTracker, SummaryState, TaskState, TaskSummaryInfo, TaskTracker,
};
pub use global_hash::{GlobalEnvConfiguration, GlobalEnvVarSummary, GlobalHashSummary};
pub use scm::SCMState;
pub use task::{
    SharedTaskSummary, SinglePackageTaskSummary, TaskCacheSummary, TaskEnvConfiguration,
    TaskEnvVarSummary, TaskExecutionSummary, TaskSummary, TaskSummaryTaskDefinition,
};
pub use task_factory::{get_external_deps_hash, Error as TaskFactoryError, TaskSummaryFactory};
pub use tracker::{Error, RunSummary, RunTracker, SinglePackageRunSummary};
// Re-export traits from turborepo-types for convenience
// These traits are defined in turborepo-types to enable proper dependency direction:
// infrastructure crates (turborepo-engine, turborepo-task-hash) can implement these
// traits without depending on this crate.
pub use turborepo_types::{
    EngineInfo, GlobalHashInputs, HashTrackerCacheHitMetadata, HashTrackerDetailedMap,
    HashTrackerInfo, RunOptsInfo,
};
