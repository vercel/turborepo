mod absolute_system_path;
mod absolute_system_path_buf;
mod relative_system_path;
mod relative_system_path_buf;
mod relative_unix_path_buf;

use std::path::{Path, PathBuf};

use path_slash::{PathBufExt, PathExt};
use thiserror::Error;

// Custom error type for path validation errors
#[derive(Debug, Error)]
pub enum PathValidationError {
    #[error("Path is non-UTF-8")]
    NonUtf8,
    #[error("Path is not absolute")]
    NotAbsolute,
    #[error("Path is not relative")]
    NotRelative,
}

trait IntoSystem {
    fn into_system(&self) -> Result<PathBuf, PathValidationError>;
}

trait IntoUnix {
    fn into_unix(&self) -> Result<PathBuf, PathValidationError>;
}

impl IntoSystem for Path {
    fn into_system(&self) -> Result<PathBuf, PathValidationError> {
        let path_str = self.to_str().ok_or(PathValidationError::NonUtf8)?;

        Ok(PathBuf::from_slash(path_str))
    }
}

impl IntoUnix for Path {
    fn into_unix(&self) -> Result<PathBuf, PathValidationError> {
        Ok(PathBuf::from(
            self.to_slash()
                .ok_or(PathValidationError::NonUtf8)?
                .as_ref(),
        ))
    }
}
