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
pub struct AbsPath(Path);

#[derive(Hash, Eq, PartialEq, Ord, PartialOrd, Clone)]
pub struct AbsPathBuf(PathBuf);

impl fmt::Debug for AbsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Debug for AbsPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl AsRef<Path> for AbsPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<AbsPath> for AbsPath {
    fn as_ref(&self) -> &AbsPath {
        self
    }
}

impl AsRef<Path> for AbsPathBuf {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<AbsPath> for AbsPathBuf {
    fn as_ref(&self) -> &AbsPath {
        self
    }
}

impl Deref for AbsPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Deref for AbsPathBuf {
    type Target = AbsPath;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.0.as_path() as *const Path as *const AbsPath) }
    }
}

impl Borrow<AbsPath> for AbsPathBuf {
    fn borrow(&self) -> &AbsPath {
        self
    }
}

impl ToOwned for AbsPath {
    type Owned = AbsPathBuf;

    fn to_owned(&self) -> Self::Owned {
        AbsPathBuf(self.0.to_owned())
    }
}

impl AbsPath {
    pub fn new(path: &Path) -> anyhow::Result<&AbsPath> {
        if path.is_absolute() {
            // SAFETY: repr transparent.
            Ok(unsafe { &*(path as *const Path as *const AbsPath) })
        } else {
            Err(AbsPathError::PathNotAbsolute(path.to_path_buf()).into())
        }
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn join<P: AsRef<Path>>(&self, other: P) -> AbsPathBuf {
        let path = self.0.join(other);
        assert!(path.is_absolute());
        AbsPathBuf(path)
    }

    pub fn parent(&self) -> Option<&AbsPath> {
        self.0.parent().map(|p| AbsPath::new(p).unwrap())
    }

    pub fn strip_prefix<P: AsRef<AbsPath>>(&self, prefix: P) -> anyhow::Result<&Path> {
        Ok(self.0.strip_prefix(prefix.as_ref())?)
    }
}

impl AbsPathBuf {
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

impl TryFrom<PathBuf> for AbsPathBuf {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        AbsPath::new(&path)?;
        Ok(AbsPathBuf(path))
    }
}

impl TryFrom<String> for AbsPathBuf {
    type Error = anyhow::Error;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        AbsPathBuf::try_from(PathBuf::from(path))
    }
}
