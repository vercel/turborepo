#![feature(assert_matches)]
#![feature(fs_try_exists)]
#![deny(clippy::all)]

//! Turborepo's path handling library.
//! Defines distinct path types for the different uses of paths in Turborepo's
//! codebase.
//!
//! - `AbsoluteSystemPath(Buf)`: a path that is absolute and uses the system's
//!   path separator. Used for interacting with the filesystem
//! - `RelativeUnixPath(Buf)`: a path that is relative and uses the unix path
//!   separator, i.e. `/`. Used when saving to a cache as a platform-independent
//!   path.
//! - `AnchoredSystemPath(Buf)`: a path that is relative to a specific directory
//!   and uses the system's path separator. Used for handling files relative to
//!   the repository root.
//!
//! NOTE: All paths contain UTF-8 strings. We use `camino` as the underlying
//! representation for system paths. For reasons why, see [the `camino` documentation](https://github.com/camino-rs/camino/).
//!
//! As in `std::path`, there are `Path` and `PathBuf` variants of each path
//! type, that indicate whether the path is borrowed or owned.
//!
//! # Validation
//!
//! When initializing a path type, it is highly recommended that you use a
//! method that validates the path. This will ensure that the path is in the
//! correct format. It's important to note that the validation will only
//! check for absolute or relative and valid Unicode. It will not check
//! if the path is system or unix, since that is not always decidable.
//! For example, is `foo\bar` a Windows system path or a Unix path?
//! It has a backslash, which is simultaneously a valid character in a
//! Unix file name and the Windows path delimiter.
//!
//! Therefore, it's very important to keep the context of the initialization
//! in mind. The only case where we do more in depth validation is for cache
//! restoration. See `AnchoredSystemPathBuf::from_system_path` for more details.
//!
//! The only case where initializing a path type without validation is
//! recommended is inside turbopath itself. But that unchecked initialization
//! should be considered unsafe
mod absolute_system_path;
mod absolute_system_path_buf;
mod anchored_system_path;
mod anchored_system_path_buf;
mod relative_unix_path;
mod relative_unix_path_buf;

use std::io;

pub use absolute_system_path::{AbsoluteSystemPath, PathRelation};
pub use absolute_system_path_buf::AbsoluteSystemPathBuf;
pub use anchored_system_path::AnchoredSystemPath;
pub use anchored_system_path_buf::AnchoredSystemPathBuf;
use camino::{Utf8Path, Utf8PathBuf};
use miette::Diagnostic;
pub use relative_unix_path::RelativeUnixPath;
pub use relative_unix_path_buf::{RelativeUnixPathBuf, RelativeUnixPathBufTestExt};
use thiserror::Error;

// Lets windows know that we're going to be reading this file sequentially
#[cfg(windows)]
pub const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x08000000;

#[derive(Debug, Error, Diagnostic)]
pub enum PathError {
    #[error("Path is non-UTF-8: {0}")]
    InvalidUnicode(String),
    #[error("Failed to convert path")]
    FromPathBufError(#[from] camino::FromPathBufError),
    #[error("Failed to convert path")]
    FromPathError(#[from] camino::FromPathError),
    #[error("path is malformed: {0}")]
    MalformedPath(String),
    #[error("Path is not safe for windows: {0}")]
    WindowsUnsafePath(String),
    #[error("Path is not absolute: {0}")]
    NotAbsolute(String),
    #[error("Path is not relative: {0}")]
    NotRelative(String),
    #[error("Path {0} is not parent of {1}")]
    NotParent(String, String),
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

pub trait IntoUnix {
    fn into_unix(self) -> Utf8PathBuf;
}

#[cfg(windows)]
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

#[derive(Debug, PartialEq)]
struct PathValidation {
    well_formed: bool,
    windows_safe: bool,
}

// Checks if path is well formed and safe for Windows.
pub(crate) fn check_path(name: &str) -> PathValidation {
    if name.is_empty() {
        return PathValidation {
            well_formed: false,
            windows_safe: false,
        };
    }

    let mut well_formed = true;
    let mut windows_safe = true;

    // Name is:
    // - "."
    // - ".."
    if well_formed && (name == "." || name == "..") {
        well_formed = false;
    }

    // Name starts with:
    // - `/`
    // - `./`
    // - `../`
    if well_formed && (name.starts_with('/') || name.starts_with("./") || name.starts_with("../")) {
        well_formed = false;
    }

    // Name ends in:
    // - `/.`
    // - `/..`
    if well_formed && (name.ends_with("/.") || name.ends_with("/..")) {
        well_formed = false;
    }

    // Name contains:
    // - `//`
    // - `/./`
    // - `/../`
    if well_formed && (name.contains("//") || name.contains("/./") || name.contains("/../")) {
        well_formed = false;
    }

    // Name contains: `\`
    if name.contains('\\') {
        windows_safe = false;
    }

    PathValidation {
        well_formed,
        windows_safe,
    }
}

pub enum UnknownPathType {
    Absolute(AbsoluteSystemPathBuf),
    Anchored(AnchoredSystemPathBuf),
}

/// Categorizes a path as either an `AbsoluteSystemPathBuf` or
/// an `AnchoredSystemPathBuf` depending on whether it
/// is absolute or relative.
pub fn categorize(path: &Utf8Path) -> UnknownPathType {
    let path = Utf8PathBuf::try_from(path_clean::clean(path))
        .expect("path cleaning should preserve UTF-8");
    if path.is_absolute() {
        UnknownPathType::Absolute(AbsoluteSystemPathBuf(path))
    } else {
        UnknownPathType::Anchored(AnchoredSystemPathBuf(path))
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::{check_path, IntoUnix, PathValidation};

    #[test]
    fn test_into_unix() {
        #[cfg(unix)]
        {
            assert_eq!("foo/bar".into_unix(), "foo/bar");
            assert_eq!("/foo/bar".into_unix(), "/foo/bar");
            assert_eq!("foo\\bar".into_unix(), "foo\\bar");
        }

        #[cfg(windows)]
        {
            assert_eq!("foo/bar".into_unix(), "foo/bar");
            assert_eq!("\\foo\\bar".into_unix(), "/foo/bar");
            assert_eq!("foo\\bar".into_unix(), "foo/bar");
        }
    }

    #[test_case("", PathValidation { well_formed: false, windows_safe: false } ; "1")]
    #[test_case(".", PathValidation { well_formed: false, windows_safe: true } ; "2")]
    #[test_case("..", PathValidation { well_formed: false, windows_safe: true } ; "3")]
    #[test_case("/", PathValidation { well_formed: false, windows_safe: true } ; "4")]
    #[test_case("./", PathValidation { well_formed: false, windows_safe: true } ; "5")]
    #[test_case("../", PathValidation { well_formed: false, windows_safe: true } ; "6")]
    #[test_case("/a", PathValidation { well_formed: false, windows_safe: true } ; "7")]
    #[test_case("./a", PathValidation { well_formed: false, windows_safe: true } ; "8")]
    #[test_case("../a", PathValidation { well_formed: false, windows_safe: true } ; "9")]
    #[test_case("/.", PathValidation { well_formed: false, windows_safe: true } ; "10")]
    #[test_case("/..", PathValidation { well_formed: false, windows_safe: true } ; "11")]
    #[test_case("a/.", PathValidation { well_formed: false, windows_safe: true } ; "12")]
    #[test_case("a/..", PathValidation { well_formed: false, windows_safe: true } ; "13")]
    #[test_case("//", PathValidation { well_formed: false, windows_safe: true } ; "14")]
    #[test_case("/./", PathValidation { well_formed: false, windows_safe: true } ; "15")]
    #[test_case("/../", PathValidation { well_formed: false, windows_safe: true } ; "16")]
    #[test_case("a//", PathValidation { well_formed: false, windows_safe: true } ; "17")]
    #[test_case("a/./", PathValidation { well_formed: false, windows_safe: true } ; "18")]
    #[test_case("a/../", PathValidation { well_formed: false, windows_safe: true } ; "19")]
    #[test_case("//a", PathValidation { well_formed: false, windows_safe: true } ; "20")]
    #[test_case("/./a", PathValidation { well_formed: false, windows_safe: true } ; "21")]
    #[test_case("/../a", PathValidation { well_formed: false, windows_safe: true } ; "22")]
    #[test_case("a//a", PathValidation { well_formed: false, windows_safe: true } ; "23")]
    #[test_case("a/./a", PathValidation { well_formed: false, windows_safe: true } ; "24")]
    #[test_case("a/../a", PathValidation { well_formed: false, windows_safe: true } ; "25")]
    #[test_case("...", PathValidation { well_formed: true, windows_safe: true } ; "26")]
    #[test_case(".../a", PathValidation { well_formed: true, windows_safe: true } ; "27")]
    #[test_case("a/...", PathValidation { well_formed: true, windows_safe: true } ; "28")]
    #[test_case("a/.../a", PathValidation { well_formed: true, windows_safe: true } ; "29")]
    #[test_case(".../...", PathValidation { well_formed: true, windows_safe: true } ; "30")]
    fn test_check_path(path: &'static str, expected_output: PathValidation) {
        let output = check_path(path);
        assert_eq!(output, expected_output);
    }
}
