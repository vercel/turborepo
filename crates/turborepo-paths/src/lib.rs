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
/// NOTE: All paths contain UTF-8 strings and use `camino` as the underlying
/// representation. For reasons why, see [the `camino` documentation](https://github.com/camino-rs/camino/).
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
mod anchored_system_path_buf;
mod relative_unix_path;
mod relative_unix_path_buf;

use std::io;

pub use absolute_system_path::AbsoluteSystemPath;
pub use absolute_system_path_buf::AbsoluteSystemPathBuf;
pub use anchored_system_path_buf::AnchoredSystemPathBuf;
use camino::Utf8PathBuf;
pub use relative_unix_path::RelativeUnixPath;
pub use relative_unix_path_buf::{RelativeUnixPathBuf, RelativeUnixPathBufTestExt};

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("Path is non-UTF-8: {0}")]
    InvalidUnicode(String),
    #[error("Failed to convert path")]
    FromPathBufError(#[from] camino::FromPathBufError),
    #[error("Path is not absolute: {0}")]
    NotAbsolute(String),
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
}

pub trait IntoSystem {
    fn into_system(self) -> Utf8PathBuf;
}

pub trait IntoUnix {
    fn into_unix(self) -> Utf8PathBuf;
}

// Checks if path contains a non system separator.
fn is_not_system(path: impl AsRef<str>) -> bool {
    let non_system_separator;

    #[cfg(windows)]
    {
        non_system_separator = '/';
    }

    #[cfg(not(windows))]
    {
        non_system_separator = '\\';
    }

    path.as_ref().contains(non_system_separator)
}

fn convert_separator(
    path: impl AsRef<str>,
    input_separator: char,
    output_separator: char,
) -> Utf8PathBuf {
    let path = path.as_ref();

    Utf8PathBuf::from(
        path.chars()
            .map(|c| {
                if c == input_separator {
                    output_separator
                } else {
                    c
                }
            })
            .collect::<String>(),
    )
}

impl<T: AsRef<str>> IntoSystem for T {
    fn into_system(self) -> Utf8PathBuf {
        let output;
        #[cfg(windows)]
        {
            output = convert_separator(self, '/', std::path::MAIN_SEPARATOR)
        }

        #[cfg(not(windows))]
        {
            output = convert_separator(self, '\\', std::path::MAIN_SEPARATOR)
        }

        output
    }
}

impl<T: AsRef<str>> IntoUnix for T {
    /// NOTE: `into_unix` *only* converts Windows paths to Unix paths *on* a
    /// Windows system. Do not pass a Windows path on a Unix system and
    /// assume it'll be converted.
    fn into_unix(self) -> Utf8PathBuf {
        let output;

        #[cfg(windows)]
        {
            output = convert_separator(self, std::path::MAIN_SEPARATOR, '/')
        }

        #[cfg(not(windows))]
        {
            output = Utf8PathBuf::from(self.as_ref())
        }

        output
    }
}
