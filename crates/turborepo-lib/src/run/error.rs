use miette::Diagnostic;
use thiserror::Error;
use turborepo_daemon::{DaemonConnectorError, DaemonError};
use turborepo_engine::GraphVisualizerError;
use turborepo_repository::package_graph;
use turborepo_scope::filter::ResolutionError;
use turborepo_task_hash::{global_hash, Error as TaskHashError};
use turborepo_ui::tui;

use crate::{config, engine, engine::ValidateError, opts, task_graph};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Invalid task configuration")]
    EngineValidation(#[related] Vec<ValidateError>),
    #[error(transparent)]
    Graph(#[from] GraphVisualizerError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Builder(#[from] engine::BuilderError),
    #[error(transparent)]
    Env(#[from] turborepo_env::Error),
    #[error(transparent)]
    Opts(#[from] opts::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageManager(#[from] turborepo_repository::package_manager::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PackageGraphBuilder(#[from] package_graph::builder::Error),
    #[error(transparent)]
    DaemonConnector(#[from] DaemonConnectorError),
    #[error(transparent)]
    Cache(#[from] turborepo_cache::CacheError),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Scope(#[from] ResolutionError),
    #[error(transparent)]
    GlobalHash(#[from] global_hash::Error),
    #[error(transparent)]
    TaskHash(#[from] TaskHashError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Visitor(#[from] task_graph::VisitorError),
    #[error(transparent)]
    SignalHandler(#[from] turborepo_signals::listeners::Error),
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
    #[error(transparent)]
    Tui(#[from] tui::Error),
    #[error(transparent)]
    MicroFrontends(#[from] turborepo_microfrontends::Error),
    #[error("Microfrontends proxy error: {0}")]
    Proxy(String),
    #[error(transparent)]
    ApiClient(#[from] turborepo_api_client::Error),
    #[error(transparent)]
    Scm(#[from] turborepo_scm::Error),
    #[error("Background task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("Missing root workspace")]
    MissingRootWorkspace,
    #[error("File hash task did not complete")]
    FileHashTaskIncomplete,
    #[error("Internal dependency hash task did not complete")]
    InternalDepsTaskIncomplete,
    #[error("Global file hash task did not complete")]
    GlobalFileHashTaskIncomplete,
    #[error("Affected range was not configured")]
    MissingAffectedRange,
    #[error(
        "--shard {requested} is out of range: the task graph was divided into {total} shard(s) \
         (valid range is 1..={total})"
    )]
    ShardOutOfRange { requested: usize, total: usize },
    #[error("--shard was requested but the task graph produced no shards (no tasks to run)")]
    NoShards,
}
