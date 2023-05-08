use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Workspace '{0}' not found in lockfile")]
    MissingWorkspace(String),
    #[error("No lockfile entry found for '{0}'")]
    MissingPackage(String),
    #[error("Missing version from non-workspace package: '{0}'")]
    MissingVersion(String),
    #[error("Unable to convert from json: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Turbo doesn't support npm lockfiles without a 'packages' field")]
    UnsupportedNpmVersion,
}
