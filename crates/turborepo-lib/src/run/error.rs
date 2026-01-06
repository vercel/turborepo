use miette::Diagnostic;
use thiserror::Error;
use turborepo_daemon::{DaemonConnectorError, DaemonError};
use turborepo_engine::GraphVisualizerError;
use turborepo_repository::package_graph;
use turborepo_ui::tui;

use crate::{config, engine, engine::ValidateError, opts, run::scope, task_graph, task_hash};

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
    Scope(#[from] scope::ResolutionError),
    #[error(transparent)]
    GlobalHash(#[from] task_hash::global_hash::Error),
    #[error(transparent)]
    TaskHash(#[from] task_hash::Error),
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
}
