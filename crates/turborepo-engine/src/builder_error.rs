//! Builder error types for the engine.

// Allow unused_assignments for fields used by miette's Diagnostic derive macro
// These fields are accessed via the derive macro attributes (#[label], #[source_code], #[related])
// but clippy doesn't recognize this usage pattern
#![allow(unused_assignments)]
// Allow large error types - these are diagnostic errors that need rich context
#![allow(clippy::result_large_err)]

use miette::Diagnostic;
use thiserror::Error;
use turborepo_repository::package_graph::PackageName;

use crate::{
    InvalidTaskNameError,
    builder_errors::{
        CyclicExtends, MissingPackageFromTaskError, MissingPackageTaskError,
        MissingRootTaskInTurboJsonError, MissingTaskError, MissingTurboJsonExtends,
    },
    validate::Error as ValidateError,
};

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Missing tasks in project")]
    MissingTasks(#[related] Vec<MissingTaskError>),
    #[error("No package.json found for {workspace}")]
    MissingPackageJson { workspace: PackageName },
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingRootTaskInTurboJson(Box<MissingRootTaskInTurboJsonError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingPackageFromTask(Box<MissingPackageFromTaskError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingPackageTask(Box<MissingPackageTaskError>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingTurboJsonExtends(Box<MissingTurboJsonExtends>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    CyclicExtends(Box<CyclicExtends>),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] turborepo_config::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJson(#[from] turborepo_turbo_json::Error),
    #[error("Invalid turbo.json configuration")]
    Validation {
        #[related]
        errors: Vec<turborepo_config::Error>,
    },
    #[error(transparent)]
    Graph(#[from] turborepo_graph_utils::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    InvalidTaskName(Box<InvalidTaskNameError>),
}

impl From<ValidateError> for Error {
    fn from(err: ValidateError) -> Self {
        match err {
            ValidateError::InvalidTaskName(e) => Error::InvalidTaskName(e),
        }
    }
}

impl Error {
    /// Checks if the error is a missing turbo.json configuration error
    pub fn is_missing_turbo_json(&self) -> bool {
        matches!(
            self,
            Self::Config(err) if err.is_no_turbo_json()
        ) || matches!(
            self,
            Self::TurboJson(turborepo_turbo_json::Error::NoTurboJSON)
        )
    }

    /// Alias for `is_missing_turbo_json` to match the naming in
    /// turborepo-config
    pub fn is_no_turbo_json(&self) -> bool {
        self.is_missing_turbo_json()
    }

    /// Creates an Error from validation errors, or returns Ok(()) if no errors
    pub fn from_validation(errors: Vec<turborepo_config::Error>) -> Result<(), Self> {
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Validation { errors })
        }
    }
}
