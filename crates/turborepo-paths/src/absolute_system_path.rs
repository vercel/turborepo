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
enum AbsolutePathError {
    #[error("expected an absolute path but got a relative path instead: `{0}`")]
    PathNotAbsolute(PathBuf),
    #[error("Cannot convert path to UTF-8, `{0:?}`")]
    PathCannotBeConvertedToUtf8(OsString),
}

#[derive(Hash, Eq, PartialEq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct AbsoluteSystemPath(Path);

#[derive(Hash, Eq, PartialEq, Ord, PartialOrd, Clone)]
pub struct AbsoluteSystemPathBuf(PathBuf);

impl fmt::Debug for AbsoluteSystemPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Debug for AbsoluteSystemPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl AsRef<Path> for AbsoluteSystemPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<AbsoluteSystemPath> for AbsoluteSystemPath {
    fn as_ref(&self) -> &AbsoluteSystemPath {
        self
    }
}

impl AsRef<Path> for AbsoluteSystemPathBuf {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<AbsoluteSystemPath> for AbsoluteSystemPathBuf {
    fn as_ref(&self) -> &AbsoluteSystemPath {
        self
    }
}

impl Deref for AbsoluteSystemPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Deref for AbsoluteSystemPathBuf {
    type Target = AbsoluteSystemPath;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.0.as_path() as *const Path as *const AbsoluteSystemPath) }
    }
}

impl Borrow<AbsoluteSystemPath> for AbsoluteSystemPathBuf {
    fn borrow(&self) -> &AbsoluteSystemPath {
        self
    }
}

impl ToOwned for AbsoluteSystemPath {
    type Owned = AbsoluteSystemPathBuf;

    fn to_owned(&self) -> Self::Owned {
        AbsoluteSystemPathBuf(self.0.to_owned())
    }
}

impl AbsoluteSystemPath {
    pub fn new(path: &Path) -> anyhow::Result<&AbsoluteSystemPath> {
        if path.is_absolute() {
            // SAFETY: repr transparent.
            Ok(unsafe { &*(path as *const Path as *const AbsoluteSystemPath) })
        } else {
            Err(AbsolutePathError::PathNotAbsolute(path.to_path_buf()).into())
        }
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn join<P: AsRef<Path>>(&self, other: P) -> AbsoluteSystemPathBuf {
        let path = self.0.join(other);
        assert!(path.is_absolute());
        AbsoluteSystemPathBuf(path)
    }

    pub fn parent(&self) -> Option<&AbsoluteSystemPath> {
        self.0.parent().map(|p| AbsoluteSystemPath::new(p).unwrap())
    }

    pub fn strip_prefix<P: AsRef<AbsoluteSystemPath>>(&self, prefix: P) -> anyhow::Result<&Path> {
        Ok(self.0.strip_prefix(prefix.as_ref())?)
    }
}

impl AbsoluteSystemPathBuf {
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
            .map_err(|x| AbsolutePathError::PathCannotBeConvertedToUtf8(x).into())
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

impl TryFrom<PathBuf> for AbsoluteSystemPathBuf {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        AbsoluteSystemPath::new(&path)?;
        Ok(AbsoluteSystemPathBuf(path))
    }
}

impl TryFrom<String> for AbsoluteSystemPathBuf {
    type Error = anyhow::Error;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        AbsoluteSystemPathBuf::try_from(PathBuf::from(path))
    }
}
