use std::{backtrace, sync::Arc};

use thiserror::Error;
use turborepo_errors::{Provenance, Sourced};
use turborepo_repository::package_graph;

use crate::{
    commands::{bin, generate, prune},
    daemon::DaemonError,
    rewrite_json::RewriteError,
    run,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("No command specified")]
    NoCommand(#[backtrace] backtrace::Backtrace),
    #[error("{0}")]
    Bin(#[from] bin::Error, #[backtrace] backtrace::Backtrace),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error("at least one task must be specified")]
    NoTasks(#[backtrace] backtrace::Backtrace),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    ChromeTracing(#[from] crate::tracing::Error),
    #[error(transparent)]
    BuildPackageGraph(#[from] package_graph::builder::Error),
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
    #[error(transparent)]
    Auth(#[from] turborepo_auth::Error),
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error(transparent)]
    Generate(#[from] generate::Error),
    #[error(transparent)]
    Prune(#[from] prune::Error),
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(transparent)]
    PackageManager(#[from] turborepo_repository::package_manager::Error),
    #[error(transparent)]
    Run(#[from] run::Error),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
}

impl Sourced for Error {
    fn with_provenance(self, provenance: Option<Arc<Provenance>>) -> Self {
        match self {
            Self::Path(e) => Self::Path(e.with_provenance(provenance)),
            Self::Run(e) => Self::Run(e.with_provenance(provenance)),
            _ => todo!(),
        }
    }

    fn provenance(&self) -> Option<Arc<Provenance>> {
        match self {
            Self::Path(e) => e.provenance(),
            Self::Run(e) => e.provenance(),
            _ => todo!(),
        }
    }
}
