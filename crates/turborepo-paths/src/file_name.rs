/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root
 * directory of this source tree.
 */

use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    ops::Deref,
    path::Path,
};

use compact_str::CompactString;
use derive_more::Display;
use ref_cast::RefCast;
use relative_path::RelativePath;
use thiserror::Error;

use crate::forward_relative_path::ForwardRelativePath;

/// Errors from ForwardRelativePath creation
#[derive(Error, Debug)]
enum FileNameError {
    #[error("file name is empty")]
    Empty,
    #[error("file name is current directory")]
    Dot,
    #[error("file name is parent directory")]
    DotDot,
    #[error("slashes in path: `{0}`")]
    Slashes(String),
}

fn verify_file_name(file_name: &str) -> anyhow::Result<()> {
    if file_name.is_empty() {
        Err(FileNameError::Empty.into())
    } else if file_name == "." {
        Err(FileNameError::Dot.into())
    } else if file_name == ".." {
        Err(FileNameError::DotDot.into())
    } else if file_name.contains('/') || file_name.contains('\\') {
        // Note we do not allow backslashes in file names
        // even if it is valid file name on Linux.
        Err(FileNameError::Slashes(file_name.to_owned()).into())
    } else {
        // At the moment we allow characters like '\0'
        // even if they are not valid at least on Linux.
        Ok(())
    }
}

/// File name. Cannot be empty, cannot contain slashes, '.' or '..'.
#[repr(transparent)]
#[derive(Display, Debug, RefCast, PartialOrd, Ord, Eq)]
pub struct FileName(str);

impl PartialEq<str> for FileName {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        &self.0 == other
    }
}

impl PartialEq<FileName> for str {
    #[inline]
    fn eq(&self, other: &FileName) -> bool {
        self == &other.0
    }
}

impl AsRef<Path> for FileName {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

impl AsRef<str> for FileName {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<RelativePath> for FileName {
    #[inline]
    fn as_ref(&self) -> &RelativePath {
        RelativePath::new(&self.0)
    }
}

impl AsRef<ForwardRelativePath> for FileName {
    #[inline]
    fn as_ref(&self) -> &ForwardRelativePath {
        ForwardRelativePath::unchecked_new(&self.0)
    }
}

impl FileName {
    /// Creates an `FileName` if the given path represents a correct
    /// platform-indepentent file name, otherwise error.
    ///
    /// ```
    /// use turborepo_paths::file_name::FileName;
    /// assert!(FileName::new("foo").is_ok());
    /// assert!(FileName::new("").is_err());
    /// assert!(FileName::new(".").is_err());
    /// assert!(FileName::new("..").is_err());
    /// assert!(FileName::new(".x").is_ok());
    /// assert!(FileName::new("foo/bar").is_err());
    /// assert!(FileName::new("foo/").is_err());
    /// assert!(FileName::new("foo\\bar").is_err());
    /// ```
    #[inline]
    pub fn new<S: ?Sized + AsRef<str>>(s: &S) -> anyhow::Result<&Self> {
        verify_file_name(s.as_ref())?;
        Ok(Self::unchecked_new(s.as_ref()))
    }

    #[inline]
    pub const fn unchecked_new(s: &str) -> &FileName {
        unsafe {
            // SAFETY: `FileName` is `repr(transparent)` over `str`.
            &*(s as *const str as *const FileName)
        }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Extracts the stem (non-extension) portion of [`self.file_name`].
    ///
    /// The stem is:
    ///
    /// * [`None`], if there is no file name;
    /// * The entire file name if there is no embedded `.`;
    /// * The entire file name if the file name begins with `.` and has no other
    ///   `.`s within;
    /// * Otherwise, the portion of the file name before the final `.`
    ///
    /// ```
    /// use turborepo_paths::file_name::FileName;
    ///
    /// let path = FileName::new("foo.rs")?;
    ///
    /// assert_eq!(Some("foo"), path.file_stem());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    pub fn file_stem(&self) -> Option<&str> {
        ForwardRelativePath::unchecked_new(&self.0).file_stem()
    }

    /// Extracts the extension of [`self.file_name`], if possible.
    ///
    /// ```
    /// 
    /// use turborepo_paths::file_name::FileName;
    ///
    /// assert_eq!(Some("rs"), FileName::new("foo.rs")?.extension());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    pub fn extension(&self) -> Option<&str> {
        ForwardRelativePath::unchecked_new(&self.0).extension()
    }
}

impl PartialEq for FileName {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Hash for FileName {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

impl ToOwned for FileName {
    type Owned = FileNameBuf;

    #[inline]
    fn to_owned(&self) -> FileNameBuf {
        FileNameBuf(CompactString::new(&self.0))
    }
}

/// Owned version of [`FileName`].
#[derive(Ord, PartialOrd, Eq, Display, Debug, Clone)]
pub struct FileNameBuf(CompactString);

impl FileNameBuf {
    #[inline]
    pub fn unchecked_new<T>(s: T) -> Self
    where
        T: Into<CompactString>,
    {
        // NOTE: This does not turn a String into an inlined string if it's already
        // allocated.
        Self(s.into())
    }

    #[inline]
    pub fn try_from_or_get_back(s: CompactString) -> Result<FileNameBuf, CompactString> {
        if verify_file_name(&s).is_ok() {
            Ok(FileNameBuf(s))
        } else {
            Err(s)
        }
    }

    #[inline]
    pub fn into_inner(self) -> CompactString {
        self.0
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl<T> PartialEq<T> for FileNameBuf
where
    T: AsRef<str>,
{
    #[inline]
    fn eq(&self, other: &T) -> bool {
        self.0 == other.as_ref()
    }
}

impl Hash for FileNameBuf {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_str().hash(state)
    }
}

impl Borrow<FileName> for FileNameBuf {
    #[inline]
    fn borrow(&self) -> &FileName {
        self.as_ref()
    }
}

impl Deref for FileNameBuf {
    type Target = FileName;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl Borrow<str> for FileNameBuf {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_ref()
    }
}

impl AsRef<FileName> for FileNameBuf {
    #[inline]
    fn as_ref(&self) -> &FileName {
        FileName::unchecked_new(self.0.as_str())
    }
}

impl AsRef<Path> for FileNameBuf {
    #[inline]
    fn as_ref(&self) -> &Path {
        Path::new(self.0.as_str())
    }
}

impl AsRef<str> for FileNameBuf {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<RelativePath> for FileNameBuf {
    #[inline]
    fn as_ref(&self) -> &RelativePath {
        RelativePath::new(self.0.as_str())
    }
}

impl AsRef<ForwardRelativePath> for FileNameBuf {
    #[inline]
    fn as_ref(&self) -> &ForwardRelativePath {
        ForwardRelativePath::unchecked_new(self.0.as_str())
    }
}

impl TryFrom<String> for FileNameBuf {
    type Error = anyhow::Error;

    #[inline]
    fn try_from(value: String) -> anyhow::Result<FileNameBuf> {
        // NOTE: This does not turn a String into an inlined string.
        verify_file_name(value.as_str())?;
        Ok(FileNameBuf(value.into()))
    }
}

impl TryFrom<CompactString> for FileNameBuf {
    type Error = anyhow::Error;

    #[inline]
    fn try_from(value: CompactString) -> anyhow::Result<FileNameBuf> {
        verify_file_name(value.as_str())?;
        Ok(FileNameBuf(value))
    }
}
