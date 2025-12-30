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

use std::collections::HashMap;

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
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};
use turborepo_cache::CacheHitMetadata;
use turborepo_env::{DetailedMap, EnvironmentVariableMap};
use turborepo_task_id::TaskId;
use turborepo_types::{DryRunMode, EnvMode, TaskDefinition};

/// Trait for accessing engine information (task definitions, dependencies)
pub trait EngineInfo {
    type TaskIter<'a>: Iterator<Item = &'a TaskId<'static>>
    where
        Self: 'a;

    fn task_definition(&self, task_id: &TaskId<'static>) -> Option<&TaskDefinition>;
    fn dependencies(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>>;
    fn dependents(&self, task_id: &TaskId<'static>) -> Option<Self::TaskIter<'_>>;
}

/// Trait for accessing run options
pub trait RunOptsInfo {
    fn dry_run(&self) -> Option<DryRunMode>;
    fn single_package(&self) -> bool;
    fn summarize(&self) -> Option<&str>;
    fn framework_inference(&self) -> bool;
    fn pass_through_args(&self) -> &[String];
    fn tasks(&self) -> &[String];
}

/// Trait for accessing task hash information
pub trait HashTrackerInfo {
    fn hash(&self, task_id: &TaskId) -> Option<String>;
    fn env_vars(&self, task_id: &TaskId) -> Option<DetailedMap>;
    fn cache_status(&self, task_id: &TaskId) -> Option<CacheHitMetadata>;
    fn expanded_outputs(&self, task_id: &TaskId) -> Option<Vec<AnchoredSystemPathBuf>>;
    fn framework(&self, task_id: &TaskId) -> Option<String>;
    fn expanded_inputs(&self, task_id: &TaskId) -> Option<HashMap<RelativeUnixPathBuf, String>>;
}

/// Trait for global hash inputs
pub trait GlobalHashInputs {
    fn root_key(&self) -> &str;
    fn global_cache_key(&self) -> &str;
    fn global_file_hash_map(&self) -> &HashMap<RelativeUnixPathBuf, String>;
    fn root_external_deps_hash(&self) -> &str;
    fn env(&self) -> &[String];
    fn resolved_env_vars(&self) -> Option<&EnvironmentVariableMap>;
    fn pass_through_env(&self) -> Option<&[String]>;
    fn env_mode(&self) -> EnvMode;
    fn framework_inference(&self) -> bool;
    fn dot_env(&self) -> Option<&[RelativeUnixPathBuf]>;
}
