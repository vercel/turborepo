use std::{backtrace, io};

use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;

use crate::{
    commands::{bin, prune},
    daemon::DaemonError,
    rewrite_json::RewriteError,
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
    #[error("Encountered an IO error while attempting to read {config_path}: {error}")]
    FailedToReadConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error("Encountered an IO error while attempting to set {config_path}: {error}")]
    FailedToSetConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
    #[error(transparent)]
    Auth(#[from] turborepo_auth::Error),
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    #[error(transparent)]
    Prune(#[from] prune::Error),
    // Temporary to prevent having to move all of the errors from anyhow to thiserror at once.
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}
