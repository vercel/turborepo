#![feature(assert_matches)]

/// Turborepo's path handling library
/// Defines distinct path types for the different usecases of paths in turborepo
///
/// - `AbsoluteSystemPath(Buf)`: a path that is absolute and uses the system's
///   path separator. Used for interacting with the filesystem
/// - `RelativeSystemPath(Buf)`: a path that is relative and uses the system's
///   path separator. Mostly used for appending onto `AbsoluteSystemPaths`.
/// - `RelativeUnixPath(Buf)`: a path that is relative and uses the unix path
///   separator. Used when saving to a cache as a platform-independent path.
/// - `AnchoredSystemPath(Buf)`: a path that is relative to a specific directory
///   and uses the system's path separator. Used for handling files relative to
///   the repository root.
///
/// As in `std::path`, there are `Path` and `PathBuf` variants of each path
/// type, that indicate whether the path is borrowed or owned.
///
/// When initializing a path type, it is highly recommended that you use a
/// method that validates the path. This will ensure that the path is in the
/// correct format. For the -Buf variants, the `new` method will validate that
/// the path is either absolute or relative, and then convert it to either
/// system or unix. For the non-Buf variants, the `new` method will *only*
/// validate and not convert (this is because conversion requires allocation).
///
/// The only case where initializing a path type without validation is
/// recommended is inside turbopath itself. But that unchecked initialization
/// should be considered unsafe
mod absolute_system_path;
mod absolute_system_path_buf;
mod anchored_system_path;
mod anchored_system_path_buf;
mod anchored_unix_tar_path_buf;
mod relative_unix_path;
mod relative_unix_path_buf;

use std::{
    io,
    path::{Path, PathBuf},
};

pub use absolute_system_path::AbsoluteSystemPath;
pub use absolute_system_path_buf::AbsoluteSystemPathBuf;
pub use anchored_system_path::AnchoredSystemPath;
pub use anchored_system_path_buf::AnchoredSystemPathBuf;
pub use anchored_unix_tar_path_buf::AnchoredUnixTarPathBuf;
use path_slash::{PathBufExt, PathExt};
pub use relative_unix_path::RelativeUnixPath;
pub use relative_unix_path_buf::{RelativeUnixPathBuf, RelativeUnixPathBufTestExt};

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("Path is non-UTF-8: {0}")]
    InvalidUnicode(String),
    #[error("Path is not absolute: {0}")]
    NotAbsolute(PathBuf),
    #[error("Path is not relative: {0}")]
    NotRelative(String),
    #[error("Path {0} is not parent of {1}")]
    NotParent(String, String),
    #[error("Path {0} is not a unix path")]
    NotUnix(String),
    #[error("Path {0} is not a system path")]
    NotSystem(String),
    #[error("IO Error {0}")]
    IO(#[from] io::Error),
    #[error("{0} is not a prefix for {1}")]
    PrefixError(String, String),
    #[error("Could not convert Unix path to system path: {0}")]
    FailedToConvertToSystem(#[from] bstr::Utf8Error),
}

impl From<std::string::FromUtf8Error> for PathError {
    fn from(value: std::string::FromUtf8Error) -> Self {
        PathError::InvalidUnicode(value.utf8_error().to_string())
    }
}

impl PathError {
    pub fn is_io_error(&self, kind: io::ErrorKind) -> bool {
        matches!(self, PathError::IO(err) if err.kind() == kind)
    }

    pub fn invalid_utf8_error(bytes: impl Into<Vec<u8>>) -> Self {
        Self::InvalidUnicode(std::string::String::from_utf8_lossy(&bytes.into()).into_owned())
    }

    pub(crate) fn not_relative_error(bytes: &[u8]) -> PathError {
        let s = String::from_utf8_lossy(bytes).to_string();
        PathError::NotRelative(s)
    }
}

trait IntoSystem {
    fn into_system(self) -> Result<PathBuf, PathError>;
}

trait IntoUnix {
    fn into_unix(self) -> Result<PathBuf, PathError>;
}

impl IntoSystem for &Path {
    fn into_system(self) -> Result<PathBuf, PathError> {
        let path_str = self
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(self.to_string_lossy().to_string()))?;

        Ok(PathBuf::from_slash(path_str))
    }
}

impl IntoUnix for &Path {
    /// NOTE: `into_unix` *only* converts Windows paths to Unix paths *on* a
    /// Windows system. Do not pass a Windows path on a Unix system and
    /// assume it'll be converted.
    fn into_unix(self) -> Result<PathBuf, PathError> {
        Ok(PathBuf::from(
            self.to_slash()
                .ok_or_else(|| PathError::InvalidUnicode(self.to_string_lossy().to_string()))?
                .as_ref(),
        ))
    }
}

// Lets windows know that we're going to be reading this file sequentially
#[cfg(windows)]
pub const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x08000000;
