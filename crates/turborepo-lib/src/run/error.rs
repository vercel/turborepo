use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::package_graph;
use turborepo_ui::tui;

use super::graph_visualizer;
use crate::{
    config, daemon, engine,
    engine::ValidateError,
    opts,
    run::{global_hash, scope},
    task_graph, task_hash,
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Invalid task configuration")]
    EngineValidation(#[related] Vec<ValidateError>),
    #[error(transparent)]
    Graph(#[from] graph_visualizer::Error),
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
    DaemonConnector(#[from] daemon::DaemonConnectorError),
    #[error(transparent)]
    Cache(#[from] turborepo_cache::CacheError),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Scope(#[from] scope::ResolutionError),
    #[error(transparent)]
    GlobalHash(#[from] global_hash::Error),
    #[error(transparent)]
    TaskHash(#[from] task_hash::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Visitor(#[from] task_graph::VisitorError),
    #[error(transparent)]
    SignalHandler(#[from] turborepo_signals::listeners::Error),
    #[error(transparent)]
    Daemon(#[from] daemon::DaemonError),
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
    #[error(transparent)]
    Tui(#[from] tui::Error),
    #[error("Failed to read microfrontends configuration: {0}")]
    MicroFrontends(#[from] turborepo_microfrontends::Error),
    #[error("Microfrontends proxy error: {0}")]
    Proxy(String),
}

impl turborepo_errors::Classify for Error {
    fn classify(&self) -> turborepo_errors::ErrorClassification {
        use turborepo_errors::ErrorClassification;

        match self {
            Error::EngineValidation(_) => ErrorClassification::Configuration,
            Error::Graph(_) => ErrorClassification::Internal,
            Error::Builder(_) => ErrorClassification::Configuration,
            Error::Env(_) => ErrorClassification::Environment,
            Error::Opts(_) => ErrorClassification::UserInput,
            Error::PackageJson(_) => ErrorClassification::Parsing,
            Error::PackageManager(_) => ErrorClassification::Configuration,
            Error::Config(_) => ErrorClassification::Configuration,
            Error::PackageGraphBuilder(_) => ErrorClassification::Configuration,
            Error::DaemonConnector(_) => ErrorClassification::Daemon,
            Error::Cache(_) => ErrorClassification::Cache,
            Error::Path(_) => ErrorClassification::FileSystem,
            Error::Scope(_) => ErrorClassification::UserInput,
            Error::GlobalHash(_) => ErrorClassification::Internal,
            Error::TaskHash(_) => ErrorClassification::Internal,
            Error::Visitor(_) => ErrorClassification::Internal,
            Error::SignalHandler(_) => ErrorClassification::Internal,
            Error::Daemon(_) => ErrorClassification::Daemon,
            Error::UI(_) => ErrorClassification::Internal,
            Error::Tui(_) => ErrorClassification::Internal,
            Error::MicroFrontends(_) => ErrorClassification::Configuration,
            Error::Proxy(_) => ErrorClassification::Proxy,
        }
    }
}
