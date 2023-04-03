mod absolute_system_path;
mod absolute_system_path_buf;
mod relative_system_path;
mod relative_system_path_buf;
mod relative_unix_path_buf;

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
