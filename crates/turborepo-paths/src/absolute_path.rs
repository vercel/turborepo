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
    ffi::OsString,
    fmt,
    ops::Deref,
    path::{Path, PathBuf},
};

use thiserror::Error;

#[derive(Error, Debug)]
enum AbsPathError {
    #[error("expected an absolute path but got a relative path instead: `{0}`")]
    PathNotAbsolute(PathBuf),
    #[error("Cannot convert path to UTF-8, `{0:?}`")]
    PathCannotBeConvertedToUtf8(OsString),
}

#[derive(Hash, Eq, PartialEq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct AbsolutePath(Path);

#[derive(Hash, Eq, PartialEq, Ord, PartialOrd, Clone)]
pub struct AbsolutePathBuf(PathBuf);

impl fmt::Debug for AbsolutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Debug for AbsolutePathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<AbsolutePath> for AbsolutePath {
    fn as_ref(&self) -> &AbsolutePath {
        self
    }
}

impl AsRef<Path> for AbsolutePathBuf {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<AbsolutePath> for AbsolutePathBuf {
    fn as_ref(&self) -> &AbsolutePath {
        self
    }
}

impl Deref for AbsolutePath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Deref for AbsolutePathBuf {
    type Target = AbsolutePath;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.0.as_path() as *const Path as *const AbsolutePath) }
    }
}

impl Borrow<AbsolutePath> for AbsolutePathBuf {
    fn borrow(&self) -> &AbsolutePath {
        self
    }
}

impl ToOwned for AbsolutePath {
    type Owned = AbsolutePathBuf;

    fn to_owned(&self) -> Self::Owned {
        AbsolutePathBuf(self.0.to_owned())
    }
}

impl AbsolutePath {
    pub fn new(path: &Path) -> anyhow::Result<&AbsolutePath> {
        if path.is_absolute() {
            // SAFETY: repr transparent.
            Ok(unsafe { &*(path as *const Path as *const AbsolutePath) })
        } else {
            Err(AbsPathError::PathNotAbsolute(path.to_path_buf()).into())
        }
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn join<P: AsRef<Path>>(&self, other: P) -> AbsolutePathBuf {
        let path = self.0.join(other);
        assert!(path.is_absolute());
        AbsolutePathBuf(path)
    }

    pub fn parent(&self) -> Option<&AbsolutePath> {
        self.0.parent().map(|p| AbsolutePath::new(p).unwrap())
    }

    pub fn strip_prefix<P: AsRef<AbsolutePath>>(&self, prefix: P) -> anyhow::Result<&Path> {
        Ok(self.0.strip_prefix(prefix.as_ref())?)
    }
}

impl AbsolutePathBuf {
    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    pub fn into_os_string(self) -> OsString {
        self.0.into_os_string()
    }

    /// Convert a path into a String. Fails if the path is not UTF8.
    pub fn into_string(self) -> anyhow::Result<String> {
        self.into_os_string()
            .into_string()
            .map_err(|x| AbsPathError::PathCannotBeConvertedToUtf8(x).into())
    }

    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional)
    }

    pub fn shrink_to_fit(&mut self) {
        self.0.shrink_to_fit()
    }

    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.0.shrink_to(min_capacity)
    }

    pub fn push<P: AsRef<Path>>(&mut self, path: P) {
        self.0.push(path);
        assert!(self.0.is_absolute());
    }

    pub fn pop(&mut self) -> bool {
        let r = self.0.pop();
        assert!(self.0.is_absolute());
        r
    }

    pub fn set_extension<S: AsRef<str>>(&mut self, extension: S) {
        self.0.set_extension(extension.as_ref());
        assert!(self.0.is_absolute());
    }
}

impl TryFrom<PathBuf> for AbsolutePathBuf {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        AbsolutePath::new(&path)?;
        Ok(AbsolutePathBuf(path))
    }
}

impl TryFrom<String> for AbsolutePathBuf {
    type Error = anyhow::Error;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        AbsolutePathBuf::try_from(PathBuf::from(path))
    }
}
