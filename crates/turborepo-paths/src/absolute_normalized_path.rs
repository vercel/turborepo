/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root
 * directory of this source tree.
 */

use std::{
    borrow::{Borrow, Cow},
    ffi::{OsStr, OsString},
    ops::Deref,
    path::{Path, PathBuf},
};

use derive_more::Display;
use ref_cast::RefCast;
use relative_path::RelativePath;
use serde::{de::Error, Deserialize, Serialize};
use thiserror::Error;

use crate::{
    absolute_path::{AbsolutePath, AbsolutePathBuf},
    forward_relative_path::{ForwardRelativePath, ForwardRelativePathNormalizer},
};

/// An absolute path. This path is not platform agnostic.
///
/// The path is normalized:
/// * it is absolute
/// * not dot in path
/// * no dot-dot in path
/// * TODO(nga): normalize slashes on Windows
/// * TODO(nga): validate UTF-8
/// * the path is **not** canonicalized
#[derive(Display, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, RefCast)]
#[display(fmt = "{}", "_0.display()")]
#[repr(transparent)]
pub struct AbsoluteNormalizedPath(AbsolutePath);

/// The owned version of [`AbsoluteNormalizedPath`].
#[derive(Clone, Display, Debug, Hash, PartialEq, Eq, Ord, PartialOrd)]
#[display(fmt = "{}", "_0.display()")]
pub struct AbsoluteNormalizedPathBuf(AbsolutePathBuf);

impl AsRef<Path> for AbsoluteNormalizedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for AbsoluteNormalizedPathBuf {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<AbsolutePath> for AbsoluteNormalizedPath {
    fn as_ref(&self) -> &AbsolutePath {
        &self.0
    }
}

impl AsRef<AbsolutePath> for AbsoluteNormalizedPathBuf {
    fn as_ref(&self) -> &AbsolutePath {
        &self.0
    }
}

impl Deref for AbsoluteNormalizedPath {
    type Target = AbsolutePath;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Serialize for AbsoluteNormalizedPathBuf {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AbsoluteNormalizedPathBuf {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        AbsoluteNormalizedPathBuf::new(PathBuf::deserialize(deserializer)?)
            .map_err(D::Error::custom)
    }
}

impl AbsoluteNormalizedPath {
    /// Creates an 'AbsPath' if the given path represents an absolute path,
    /// otherwise error.
    ///
    /// ```
    /// # use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPath;
    ///
    /// assert!(AbsoluteNormalizedPath::new("foo/bar").is_err());
    /// if cfg!(windows) {
    ///    assert!(AbsoluteNormalizedPath::new("C:\\foo\\bar").is_ok());
    /// } else {
    ///    assert!(AbsoluteNormalizedPath::new("/foo/bar").is_ok());
    /// }
    /// # anyhow::Ok(())
    /// ```
    pub fn new<P: ?Sized + AsRef<Path>>(p: &P) -> anyhow::Result<&AbsoluteNormalizedPath> {
        let path = AbsolutePath::new(p.as_ref())?;
        verify_abs_path(path)?;
        Ok(AbsoluteNormalizedPath::ref_cast(path))
    }

    /// Creates an owned 'AbsPathBuf' with path adjoined to self.
    ///
    /// ```
    /// use std::path::Path;
    /// use turborepo_paths::absolute_normalized_path::{AbsoluteNormalizedPath, AbsoluteNormalizedPathBuf};
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// if cfg!(not(windows)) {
    ///     let abs_path = AbsoluteNormalizedPath::new("/my")?;
    ///     assert_eq!(AbsoluteNormalizedPathBuf::from("/my/foo/bar".into())?, abs_path.join(ForwardRelativePath::new("foo/bar")?));
    /// } else {
    ///     let abs_path = AbsoluteNormalizedPath::new("C:\\my")?;
    ///     assert_eq!("C:\\my\\foo\\bar", abs_path.join(ForwardRelativePath::new("foo/bar")?).to_string());
    /// }
    /// # anyhow::Ok(())
    /// ```
    #[allow(clippy::collapsible_else_if)]
    pub fn join<P: AsRef<ForwardRelativePath>>(&self, path: P) -> AbsoluteNormalizedPathBuf {
        let path = path.as_ref();
        if path.is_empty() {
            self.to_buf()
        } else {
            if cfg!(windows) {
                AbsoluteNormalizedPathBuf(self.0.join(path.as_str().replace('/', "\\")))
            } else {
                AbsoluteNormalizedPathBuf(self.0.join(path.as_str()))
            }
        }
    }

    /// Returns a relative path of the parent directory
    ///
    /// ```
    /// use std::path::Path;
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPath;
    ///
    /// if cfg!(not(windows)) {
    ///     assert_eq!(
    ///         Some(AbsoluteNormalizedPath::new("/")?),
    ///         AbsoluteNormalizedPath::new("/my")?.parent()
    ///     );
    ///     assert_eq!(
    ///         None,
    ///         AbsoluteNormalizedPath::new("/")?.parent()
    ///     );
    /// } else {
    ///     assert_eq!(
    ///         Some(AbsoluteNormalizedPath::new("c:/")?),
    ///         AbsoluteNormalizedPath::new("c:/my")?.parent()
    ///     );
    ///     assert_eq!(
    ///         None,
    ///         AbsoluteNormalizedPath::new("c:/")?.parent()
    ///     );
    /// }
    /// # anyhow::Ok(())
    /// ```
    pub fn parent(&self) -> Option<&AbsoluteNormalizedPath> {
        self.0.parent().map(AbsoluteNormalizedPath::ref_cast)
    }

    /// Returns a 'ForwardRelativePath' that, when joined onto `base`, yields
    /// `self`.
    ///
    /// Error if `base` is not a prefix of `self` or the returned
    /// path is not a 'ForwardRelativePath'
    ///
    /// ```
    /// use std::{borrow::Cow, path::Path};
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPath;
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// if cfg!(not(windows)) {
    ///     let path = AbsoluteNormalizedPath::new("/test/foo/bar.txt")?;
    ///
    ///     assert_eq!(
    ///         path.strip_prefix(AbsoluteNormalizedPath::new("/test")?)?,
    ///         Cow::Borrowed(ForwardRelativePath::new("foo/bar.txt")?)
    ///     );
    ///     assert!(path.strip_prefix(AbsoluteNormalizedPath::new("/asdf")?).is_err());
    /// } else {
    ///     let path = AbsoluteNormalizedPath::new(r"C:\test\foo\bar.txt")?;
    ///
    ///     // strip_prefix will return Cow::Owned here but we still
    ///     // can compare it to Cow::Borrowed.
    ///     assert_eq!(
    ///         path.strip_prefix(AbsoluteNormalizedPath::new("c:/test")?)?,
    ///         Cow::Borrowed(ForwardRelativePath::new("foo/bar.txt")?)
    ///     );
    ///     assert_eq!(
    ///         path.strip_prefix(AbsoluteNormalizedPath::new(r"c:\test")?)?,
    ///         Cow::Borrowed(ForwardRelativePath::new("foo/bar.txt")?)
    ///     );
    ///     assert_eq!(
    ///         path.strip_prefix(AbsoluteNormalizedPath::new(r"\\?\c:\test")?)?,
    ///         Cow::Borrowed(ForwardRelativePath::new("foo/bar.txt")?)
    ///     );
    ///     assert!(path.strip_prefix(AbsoluteNormalizedPath::new("c:/asdf")?).is_err());
    ///
    ///     let shared_path = AbsoluteNormalizedPath::new(r"\\server\share\foo\bar.txt")?;
    ///     assert_eq!(
    ///         shared_path.strip_prefix(AbsoluteNormalizedPath::new(r"\\server\share\")?)?,
    ///         Cow::Borrowed(ForwardRelativePath::new("foo/bar.txt")?)
    ///     );
    ///     assert_eq!(
    ///         shared_path.strip_prefix(AbsoluteNormalizedPath::new(r"\\server\share\foo")?)?,
    ///         Cow::Borrowed(ForwardRelativePath::new("bar.txt")?)
    ///     );
    ///     assert_eq!(
    ///         shared_path.strip_prefix(AbsoluteNormalizedPath::new(r"\\?\UNC\server\share\foo")?)?,
    ///         Cow::Borrowed(ForwardRelativePath::new("bar.txt")?)
    ///     );
    ///     assert!(shared_path.strip_prefix(AbsoluteNormalizedPath::new(r"\\server\share2\foo")?).is_err());
    ///     assert!(shared_path.strip_prefix(AbsoluteNormalizedPath::new(r"\\server\share\fo")?).is_err());
    /// }
    /// # anyhow::Ok(())
    /// ```
    pub fn strip_prefix<P: AsRef<AbsoluteNormalizedPath>>(
        &self,
        base: P,
    ) -> anyhow::Result<Cow<ForwardRelativePath>> {
        let stripped_path = self.strip_prefix_impl(base.as_ref())?;
        ForwardRelativePathNormalizer::normalize_path(stripped_path)
    }

    #[cfg(not(windows))]
    fn strip_prefix_impl(&self, base: &AbsoluteNormalizedPath) -> anyhow::Result<&Path> {
        self.0.strip_prefix(&base.0).map_err(anyhow::Error::from)
    }

    #[cfg(windows)]
    fn strip_prefix_impl(&self, base: &AbsoluteNormalizedPath) -> anyhow::Result<&Path> {
        if self.windows_prefix()? == base.windows_prefix()? {
            self.strip_windows_prefix()?
                .strip_prefix(base.strip_windows_prefix()?)
                .map_err(anyhow::Error::from)
        } else {
            Err(anyhow::anyhow!("Path is not a prefix"))
        }
    }

    /// Determines whether `base` is a prefix of `self`.
    ///
    /// ```
    /// use std::path::Path;
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPath;
    ///
    /// if cfg!(not(windows)) {
    ///     let abs_path = AbsoluteNormalizedPath::new("/some/foo")?;
    ///     assert!(abs_path.starts_with(AbsoluteNormalizedPath::new("/some")?));
    ///     assert!(!abs_path.starts_with(AbsoluteNormalizedPath::new("/som")?));
    /// } else {
    ///     let abs_path = AbsoluteNormalizedPath::new("c:/some/foo")?;
    ///     assert!(abs_path.starts_with(AbsoluteNormalizedPath::new("c:/some")?));
    ///     assert!(!abs_path.starts_with(AbsoluteNormalizedPath::new("c:/som")?));
    ///     assert!(abs_path.starts_with(AbsoluteNormalizedPath::new(r"\\?\C:\some")?));
    ///
    ///     let shared_path = AbsoluteNormalizedPath::new(r"\\server\share\foo\bar.txt")?;
    ///     assert!(shared_path.starts_with(AbsoluteNormalizedPath::new(r"\\server\share\")?));
    ///     assert!(shared_path.starts_with(AbsoluteNormalizedPath::new(r"\\server\share\foo")?));
    ///     assert!(shared_path.starts_with(AbsoluteNormalizedPath::new(r"\\?\UNC\server\share\foo")?));
    ///     assert!(!shared_path.starts_with(AbsoluteNormalizedPath::new(r"\\server\share2\foo")?));
    ///     assert!(!shared_path.starts_with(AbsoluteNormalizedPath::new(r"\\server\share\fo")?));
    /// }
    /// # anyhow::Ok(())
    /// ```
    pub fn starts_with<P: AsRef<AbsoluteNormalizedPath>>(&self, base: P) -> bool {
        self.starts_with_impl(base.as_ref())
    }

    #[cfg(not(windows))]
    fn starts_with_impl(&self, base: &AbsoluteNormalizedPath) -> bool {
        self.0.starts_with(&base.0)
    }

    #[cfg(windows)]
    fn starts_with_impl(&self, base: &AbsoluteNormalizedPath) -> bool {
        let prefix = self.windows_prefix();
        let base_prefix = base.windows_prefix();
        if let (Ok(prefix), Ok(base_prefix)) = (prefix, base_prefix) {
            if prefix == base_prefix {
                let stripped = self.strip_windows_prefix();
                let base_stripped = base.strip_windows_prefix();
                if let (Ok(stripped), Ok(base_stripped)) = (stripped, base_stripped) {
                    return stripped.starts_with(base_stripped);
                }
            }
        }
        false
    }

    /// Determines whether `child` is a suffix of `self`.
    /// Only considers whole path components to match.
    ///
    /// ```
    /// use std::path::Path;
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPath;
    ///
    /// if cfg!(not(windows)) {
    ///     let abs_path = AbsoluteNormalizedPath::new("/some/foo")?;
    ///     assert!(abs_path.ends_with("foo"));
    /// } else {
    ///     let abs_path = AbsoluteNormalizedPath::new("c:/some/foo")?;
    ///     assert!(abs_path.ends_with("foo"));
    /// }
    /// # anyhow::Ok(())    
    /// ```
    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.0.ends_with(child.as_ref())
    }

    /// Build an owned `AbsPathBuf`, joined with the given path and normalized.
    ///
    /// ```
    /// use turborepo_paths::absolute_normalized_path::{AbsoluteNormalizedPath, AbsoluteNormalizedPathBuf};
    ///
    /// if cfg!(not(windows)) {
    ///     assert_eq!(
    ///         AbsoluteNormalizedPathBuf::from("/foo/baz.txt".into())?,
    ///         AbsoluteNormalizedPath::new("/foo/bar")?.join_normalized("../baz.txt")?
    ///     );
    ///
    ///     assert_eq!(
    ///         AbsoluteNormalizedPath::new("/foo")?.join_normalized("../../baz.txt").is_err(),
    ///         true
    ///     );
    /// } else {
    ///     assert_eq!(
    ///         AbsoluteNormalizedPathBuf::from("c:/foo/baz.txt".into())?,
    ///         AbsoluteNormalizedPath::new("c:/foo/bar")?.join_normalized("../baz.txt")?
    ///     );
    ///
    ///     assert_eq!(
    ///         AbsoluteNormalizedPath::new("c:/foo")?.join_normalized("../../baz.txt").is_err(),
    ///         true
    ///     );
    /// }
    /// # anyhow::Ok(())
    /// ```
    pub fn join_normalized<P: AsRef<RelativePath>>(
        &self,
        path: P,
    ) -> anyhow::Result<AbsoluteNormalizedPathBuf> {
        let mut stack = Vec::new();
        for c in self
            .0
            .components()
            .chain(path.as_ref().components().map(|c| match c {
                relative_path::Component::Normal(s) => std::path::Component::Normal(OsStr::new(s)),
                relative_path::Component::CurDir => std::path::Component::CurDir,
                relative_path::Component::ParentDir => std::path::Component::ParentDir,
            }))
        {
            match c {
                std::path::Component::Normal(_) => stack.push(c),
                std::path::Component::Prefix(_) => stack.push(c),
                std::path::Component::RootDir => stack.push(c),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if stack.pop().is_none() {
                        return Err(anyhow::anyhow!(PathNormalizationError::OutOfBounds(
                            self.as_os_str().into(),
                            path.as_ref().as_str().into(),
                        )));
                    }
                }
            }
        }
        let path_buf = stack.iter().collect::<PathBuf>();

        AbsoluteNormalizedPathBuf::try_from(path_buf)
    }

    /// Convert to an owned [`AbsoluteNormalizedPathBuf`].
    pub fn to_buf(&self) -> AbsoluteNormalizedPathBuf {
        self.to_owned()
    }

    #[cfg(windows)]
    /// Get Windows path prefix which is either disk drive letter, device or UNC
    /// name.
    ///
    /// ```
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPath;
    ///
    /// assert_eq!("D", AbsoluteNormalizedPath::new("d:/foo/bar")?.windows_prefix()?);
    /// assert_eq!("D", AbsoluteNormalizedPath::new(r"D:\foo\bar")?.windows_prefix()?);
    /// assert_eq!("E", AbsoluteNormalizedPath::new(r"\\?\E:\foo\bar")?.windows_prefix()?);
    /// assert_eq!("server\\share", AbsoluteNormalizedPath::new(r"\\server\share")?.windows_prefix()?);
    /// assert_eq!("server\\share", AbsoluteNormalizedPath::new(r"\\server\share\foo\bar")?.windows_prefix()?);
    /// assert_eq!("server\\share", AbsoluteNormalizedPath::new(r"\\?\UNC\server\share")?.windows_prefix()?);
    /// assert_eq!("COM42", AbsoluteNormalizedPath::new(r"\\.\COM42")?.windows_prefix()?);
    /// assert_eq!("COM42", AbsoluteNormalizedPath::new(r"\\.\COM42\foo\bar")?.windows_prefix()?);
    /// ```
    pub fn windows_prefix(&self) -> anyhow::Result<OsString> {
        use std::{os::windows::ffi::OsStringExt, path::Prefix};

        match self
            .0
            .components()
            .next()
            .ok_or_else(|| anyhow::anyhow!("AbsPath is empty."))?
        {
            std::path::Component::Prefix(prefix_component) => match prefix_component.kind() {
                Prefix::Disk(disk) | Prefix::VerbatimDisk(disk) => {
                    Ok(OsString::from_wide(&[disk.into()]))
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    let mut server = server.to_owned();
                    server.push("\\");
                    server.push(share);
                    Ok(server)
                }
                Prefix::DeviceNS(device) => Ok(device.to_owned()),
                prefix => Err(anyhow::anyhow!("Unknown prefix kind: {:?}.", prefix)),
            },
            _ => Err(anyhow::anyhow!("AbsPath doesn't have prefix.")),
        }
    }

    #[cfg(windows)]
    /// Strip Windows path prefix which is either disk drive letter, device or
    /// UNC name.
    ///
    /// ```
    /// use std::path::Path;
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPath;
    ///
    /// assert_eq!(Path::new(""), AbsoluteNormalizedPath::new("C:/")?.strip_windows_prefix()?);
    /// assert_eq!(Path::new(""), AbsoluteNormalizedPath::new("C:\\")?.strip_windows_prefix()?);
    /// assert_eq!(Path::new("foo/bar"), AbsoluteNormalizedPath::new("d:/foo/bar")?.strip_windows_prefix()?);
    /// assert_eq!(Path::new("foo\\bar"), AbsoluteNormalizedPath::new(r"D:\foo\bar")?.strip_windows_prefix()?);
    /// assert_eq!(Path::new("foo\\bar"), AbsoluteNormalizedPath::new(r"\\?\D:\foo\bar")?.strip_windows_prefix()?);
    /// assert_eq!(Path::new("path"), AbsoluteNormalizedPath::new(r"\\server\share\path")?.strip_windows_prefix()?);
    /// assert_eq!(Path::new("path"), AbsoluteNormalizedPath::new(r"\\?\UNC\server\share\path")?.strip_windows_prefix()?);
    /// assert_eq!(Path::new("abc"), AbsoluteNormalizedPath::new(r"\\.\COM42\abc")?.strip_windows_prefix()?);
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn strip_windows_prefix(&self) -> anyhow::Result<&Path> {
        let mut iter = self.0.iter();
        let prefix = iter
            .next()
            .ok_or_else(|| anyhow::anyhow!("AbsPath is empty."))?;
        let mut prefix = prefix.to_owned();
        // Strip leading path separator as well.
        if let Some(component) = iter.next() {
            prefix.push(component);
        }
        Ok(self.as_path().strip_prefix(&prefix)?)
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }

    pub fn as_abs_path(&self) -> &AbsolutePath {
        &self.0
    }
}

impl AbsoluteNormalizedPathBuf {
    pub fn new(path: PathBuf) -> anyhow::Result<AbsoluteNormalizedPathBuf> {
        let path = AbsolutePathBuf::try_from(path)?;
        verify_abs_path(&path)?;
        Ok(AbsoluteNormalizedPathBuf(path))
    }

    pub(crate) fn unchecked_new(path: PathBuf) -> Self {
        AbsoluteNormalizedPathBuf(AbsolutePathBuf::try_from(path).unwrap())
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0.into_path_buf()
    }

    pub fn into_abs_path_buf(self) -> AbsolutePathBuf {
        self.0
    }

    pub fn from(s: String) -> anyhow::Result<Self> {
        AbsoluteNormalizedPathBuf::try_from(s)
    }

    /// Creates a new 'AbsPathBuf' with a given capacity used to create the
    /// internal 'String'. See 'with_capacity' defined on 'PathBuf'
    pub fn with_capacity<P: AsRef<AbsoluteNormalizedPath>>(cap: usize, base: P) -> Self {
        let mut path = PathBuf::with_capacity(cap);
        path.push(base.as_ref());
        AbsoluteNormalizedPathBuf(AbsolutePathBuf::try_from(path).unwrap())
    }

    /// Returns the capacity of the underlying 'PathBuf'
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    /// Invokes 'reserve' on the underlying 'PathBuf'
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional)
    }

    /// Invokes 'shrink_to_fit' on the underlying 'PathBuf'
    pub fn shrink_to_fit(&mut self) {
        self.0.shrink_to_fit()
    }

    /// Invokes 'shrink_to' on the underlying 'PathBuf'
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.0.shrink_to(min_capacity)
    }

    /// Pushes a `ForwardRelativePath` to the existing buffer
    /// ```
    /// 
    /// use std::path::PathBuf;
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPathBuf;
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// let prefix = if cfg!(windows) {
    ///    "C:"
    /// } else {
    ///   ""
    /// };
    ///
    /// let mut path = AbsoluteNormalizedPathBuf::try_from(format!("{prefix}/foo")).unwrap();
    /// path.push(ForwardRelativePath::unchecked_new("bar"));
    ///
    /// assert_eq!(AbsoluteNormalizedPathBuf::try_from(format!("{prefix}/foo/bar")).unwrap(), path);
    ///
    /// path.push(ForwardRelativePath::unchecked_new("more/file.rs"));
    /// assert_eq!(AbsoluteNormalizedPathBuf::try_from(format!("{prefix}/foo/bar/more/file.rs")).unwrap(), path);
    /// ```
    pub fn push<P: AsRef<ForwardRelativePath>>(&mut self, path: P) {
        if cfg!(windows) {
            self.0.push(path.as_ref().as_str().replace('/', "\\"))
        } else {
            self.0.push(path.as_ref().as_str())
        }
    }

    /// Pushes a `RelativePath` to the existing buffer, normalizing it.
    /// Note that this does not visit the filesystem to resolve `..`s. Instead,
    /// it cancels out the components directly, similar to
    /// `join_normalized`. ```
    ///
    /// use turborepo_paths::abs_norm_path::AbsNormPathBuf;
    /// use turborepo_paths::RelativePath;
    ///
    /// let prefix = if cfg!(windows) {
    ///   "C:"
    /// } else {
    ///  ""
    /// };
    ///
    /// let mut path =
    /// AbsNormPathBuf::try_from(format!("{prefix}/foo")).unwrap();
    /// path.push_normalized(RelativePath::new("bar"))?;
    ///
    /// assert_eq!(AbsNormPathBuf::try_from(format!("{prefix}/foo/bar")).
    /// unwrap(), path);
    ///
    /// path.push_normalized(RelativePath::new("more/file.rs"))?;
    /// assert_eq!(AbsNormPathBuf::try_from(format!("{prefix}/foo/bar/more/file.
    /// rs")).unwrap(), path);
    ///
    /// path.push_normalized(RelativePath::new("../other.rs"))?;
    /// assert_eq!(AbsNormPathBuf::try_from(format!("{prefix}/foo/bar/more/
    /// other.rs")).unwrap(), path);
    ///
    /// path.push_normalized(RelativePath::new(".."))?;
    /// assert_eq!(AbsNormPathBuf::try_from(format!("{prefix}/foo/bar/more")).
    /// unwrap(), path);
    ///
    /// path.push_normalized(RelativePath::new("../.."))?;
    /// assert_eq!(AbsNormPathBuf::try_from(format!("{prefix}/foo")).unwrap(),
    /// path);
    ///
    /// path.push_normalized(RelativePath::new(".."))?;
    /// assert_eq!(AbsNormPathBuf::try_from(format!("{prefix}/")).unwrap(),
    /// path);
    ///
    /// assert!(path.push_normalized(RelativePath::new("..")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn push_normalized<P: AsRef<RelativePath>>(&mut self, path: P) -> anyhow::Result<()> {
        for c in path.as_ref().components() {
            match c {
                relative_path::Component::Normal(s) => {
                    self.0.push(s);
                }
                relative_path::Component::CurDir => {}
                relative_path::Component::ParentDir => {
                    if !self.0.pop() {
                        return Err(anyhow::anyhow!(PathNormalizationError::OutOfBounds(
                            self.as_os_str().into(),
                            path.as_ref().as_str().into(),
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    pub fn pop(&mut self) -> bool {
        self.0.pop()
    }
}

impl TryFrom<String> for AbsoluteNormalizedPathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPathBuf;
    /// use std::convert::TryFrom;
    ///
    /// assert!(AbsoluteNormalizedPathBuf::try_from("relative/bar".to_owned()).is_err());
    ///
    /// if cfg!(not(windows)) {
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("/foo/bar".to_owned()).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("/".to_owned()).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("/normalize/./bar".to_owned()).is_err());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("/normalize/../bar".to_owned()).is_err());
    /// } else {
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("c:/foo/bar".to_owned()).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("c:/".to_owned()).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("c:/normalize/./bar".to_owned()).is_err());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from("c:/normalize/../bar".to_owned()).is_err());
    /// }
    /// ```
    fn try_from(s: String) -> anyhow::Result<AbsoluteNormalizedPathBuf> {
        AbsoluteNormalizedPathBuf::try_from(OsString::from(s))
    }
}

impl TryFrom<OsString> for AbsoluteNormalizedPathBuf {
    type Error = anyhow::Error;

    // no allocation
    fn try_from(s: OsString) -> anyhow::Result<AbsoluteNormalizedPathBuf> {
        AbsoluteNormalizedPathBuf::try_from(PathBuf::from(s))
    }
}

impl TryFrom<PathBuf> for AbsoluteNormalizedPathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::absolute_normalized_path::AbsoluteNormalizedPathBuf;
    /// use std::convert::TryFrom;
    /// use std::path::PathBuf;
    ///
    /// assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("relative/bar")).is_err());
    ///
    /// if cfg!(not(windows)) {
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("/foo/bar")).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("/")).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("/normalize/./bar")).is_err());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("/normalize/../bar")).is_err());
    /// } else {
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("c:/foo/bar")).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("c:/")).is_ok());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("c:/normalize/./bar")).is_err());
    ///     assert!(AbsoluteNormalizedPathBuf::try_from(PathBuf::from("c:/normalize/../bar")).is_err());
    /// }
    /// ```
    fn try_from(p: PathBuf) -> anyhow::Result<AbsoluteNormalizedPathBuf> {
        let p = AbsolutePathBuf::try_from(p)?;
        verify_abs_path(&p)?;
        Ok(AbsoluteNormalizedPathBuf(p))
    }
}

impl ToOwned for AbsoluteNormalizedPath {
    type Owned = AbsoluteNormalizedPathBuf;

    fn to_owned(&self) -> AbsoluteNormalizedPathBuf {
        AbsoluteNormalizedPathBuf(self.0.to_owned())
    }
}

impl AsRef<AbsoluteNormalizedPath> for AbsoluteNormalizedPath {
    fn as_ref(&self) -> &AbsoluteNormalizedPath {
        self
    }
}

impl AsRef<AbsoluteNormalizedPath> for AbsoluteNormalizedPathBuf {
    fn as_ref(&self) -> &AbsoluteNormalizedPath {
        AbsoluteNormalizedPath::ref_cast(&self.0)
    }
}

impl Borrow<AbsoluteNormalizedPath> for AbsoluteNormalizedPathBuf {
    fn borrow(&self) -> &AbsoluteNormalizedPath {
        self.as_ref()
    }
}

impl Deref for AbsoluteNormalizedPathBuf {
    type Target = AbsoluteNormalizedPath;

    fn deref(&self) -> &AbsoluteNormalizedPath {
        AbsoluteNormalizedPath::ref_cast(&self.0)
    }
}

// Separate function so windows path verification can be tested on Unix.
fn verify_abs_path_windows_part(path: &str) -> bool {
    // UNC device path.
    // TODO(nga): behavior of UNC paths is under-specified in `AbsPath`.
    let path = path.strip_prefix("\\\\.\\").unwrap_or(path);

    for component in path.split(|c| c == '/' || c == '\\') {
        if component == "." || component == ".." {
            return false;
        }
    }

    true
}

/// Verifier for AbsPath to ensure the path is absolute
fn verify_abs_path(path: &AbsolutePath) -> anyhow::Result<()> {
    // `Path::components` normalizes '.'s away so we cannot iterate with it.
    // TODO maybe we actually want to allow "."s and just
    //   normalize them away entirely.

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        // `as_bytes` function is not available on Windows.
        // This is equivalent but faster to generic code below.
        for component in path.as_os_str().as_bytes().split(|c| *c == b'/') {
            if component == b"." || component == b".." {
                return Err(anyhow::anyhow!(AbsNormPathError::PathNotNormalized(
                    path.to_owned()
                )));
            }
        }
    }

    if !cfg!(unix) {
        let path_str = path.to_string_lossy();
        if !verify_abs_path_windows_part(&path_str) {
            return Err(anyhow::anyhow!(AbsNormPathError::PathNotNormalized(
                path.to_owned()
            )));
        }
    }

    Ok(())
}

/// Errors from 'AbsPath' creation
#[derive(Error, Debug)]
enum AbsNormPathError {
    #[error("expected a normalized path, but found a non-normalized path instead: `{0}`")]
    PathNotNormalized(AbsolutePathBuf),
}

/// Errors from normalizing paths
#[derive(Error, Debug)]
enum PathNormalizationError {
    #[error(
        "no such path: normalizing `{}` requires the parent directory of the root of `{}`",
        .1.to_string_lossy(),
        .0.to_string_lossy()
    )]
    OutOfBounds(OsString, OsString),
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
    };

    use crate::{
        absolute_normalized_path::{
            verify_abs_path_windows_part, AbsoluteNormalizedPath, AbsoluteNormalizedPathBuf,
        },
        forward_relative_path::ForwardRelativePath,
    };

    #[cfg(not(windows))]
    fn make_absolute(s: &str) -> String {
        s.to_owned()
    }

    #[cfg(windows)]
    fn make_absolute(s: &str) -> String {
        let mut abs_path = "c:".to_owned();
        abs_path.push_str(s);
        abs_path
    }

    #[test]
    fn abs_paths_work_in_maps() -> anyhow::Result<()> {
        let mut map = HashMap::new();
        let foo_string = make_absolute("/foo");
        let bar_string = make_absolute("/bar");

        let p1 = AbsoluteNormalizedPath::new(foo_string.as_str())?;
        let p2 = AbsoluteNormalizedPath::new(bar_string.as_str())?;

        map.insert(p1.to_buf(), p2.to_buf());

        assert_eq!(Some(p2), map.get(p1).map(|p| p.as_ref()));

        Ok(())
    }

    #[test]
    fn abs_path_is_comparable() -> anyhow::Result<()> {
        let foo_string = make_absolute("/foo");
        let bar_string = make_absolute("/bar");
        let path1_buf = AbsoluteNormalizedPathBuf::from(foo_string.clone())?;
        let path2_buf = AbsoluteNormalizedPathBuf::from(foo_string.clone())?;
        let path3_buf = AbsoluteNormalizedPathBuf::from(bar_string.clone())?;

        let path1 = AbsoluteNormalizedPath::new(foo_string.as_str())?;
        let path2 = AbsoluteNormalizedPath::new(foo_string.as_str())?;
        let path3 = AbsoluteNormalizedPath::new(bar_string.as_str())?;

        let str2 = foo_string.as_str();
        let str3 = bar_string.as_str();
        let str_not_abs = "ble";

        let string_not_abs = "ble".to_owned();

        assert_eq!(path1_buf, path2_buf);
        assert_ne!(path1_buf, path3_buf);

        assert_eq!(path1, path2);
        assert_ne!(path1, path3);

        assert_eq!(path1_buf, path2);
        assert_ne!(path1, path3_buf);

        assert_eq!(path1_buf, str2);
        assert_ne!(path1_buf, str3);
        assert_ne!(path1_buf, str_not_abs);

        assert_eq!(path1, str2);
        assert_ne!(path1, str3);
        assert_ne!(path1, str_not_abs);

        assert_eq!(path1_buf, foo_string);
        assert_ne!(path1_buf, bar_string);
        assert_ne!(path1_buf, string_not_abs);

        assert_eq!(path1, foo_string);
        assert_ne!(path1, bar_string);
        assert_ne!(path1, string_not_abs);

        Ok(())
    }

    #[test]
    fn test_verify() {
        assert!(AbsoluteNormalizedPath::new("relative/bar").is_err());
        assert!(AbsoluteNormalizedPath::new(Path::new("relative/bar")).is_err());

        if cfg!(not(windows)) {
            assert!(AbsoluteNormalizedPath::new("/foo/bar").is_ok());
            assert!(AbsoluteNormalizedPath::new("/").is_ok());
            assert!(AbsoluteNormalizedPath::new("/normalize/./bar").is_err());
            assert!(AbsoluteNormalizedPath::new("/normalize/../bar").is_err());

            assert!(AbsoluteNormalizedPath::new(Path::new("/foo/bar")).is_ok());
            assert!(AbsoluteNormalizedPath::new(Path::new("/")).is_ok());
            assert!(AbsoluteNormalizedPath::new(Path::new("/normalize/./bar")).is_err());
            assert!(AbsoluteNormalizedPath::new(Path::new("/normalize/../bar")).is_err());
        } else {
            assert!(AbsoluteNormalizedPath::new("c:/foo/bar").is_ok());
            assert!(AbsoluteNormalizedPath::new("c:/").is_ok());
            assert!(AbsoluteNormalizedPath::new("c:/normalize/./bar").is_err());
            assert!(AbsoluteNormalizedPath::new("c:/normalize/../bar").is_err());
            assert!(AbsoluteNormalizedPath::new("c:\\normalize\\.\\bar").is_err());
            assert!(AbsoluteNormalizedPath::new("c:\\normalize\\..\\bar").is_err());
            assert!(AbsoluteNormalizedPath::new("/foo/bar").is_err());

            assert!(AbsoluteNormalizedPath::new(Path::new("c:/foo/bar")).is_ok());
            assert!(AbsoluteNormalizedPath::new(Path::new("c:/")).is_ok());
            assert!(AbsoluteNormalizedPath::new(Path::new("c:/normalize/./bar")).is_err());
            assert!(AbsoluteNormalizedPath::new(Path::new("c:/normalize/../bar")).is_err());

            // UNC paths.
            assert!(AbsoluteNormalizedPath::new(Path::new(r"\\.\COM42")).is_ok());
            assert!(AbsoluteNormalizedPath::new(Path::new(r"\\?\c:\test")).is_ok());
        }
    }

    #[test]
    fn test_verify_windows() {
        assert!(verify_abs_path_windows_part(r"c:\foo\bar"));
        assert!(verify_abs_path_windows_part(r"\\.\COM42"));
        assert!(verify_abs_path_windows_part(r"\\?\c:\test"));
        assert!(!verify_abs_path_windows_part(r"\\?\c:\.\test"));
    }

    #[test]
    fn test_pop() {
        let mut path = if cfg!(not(windows)) {
            PathBuf::from("/foo/bar")
        } else {
            PathBuf::from("c:/foo/bar")
        };
        let mut abs_path = AbsoluteNormalizedPath::new(&path).unwrap().to_buf();

        assert!(path.pop());
        assert!(abs_path.pop());
        assert_eq!(path, abs_path.as_path());

        assert!(path.pop());
        assert!(abs_path.pop());
        assert_eq!(path, abs_path.as_path());

        assert!(!path.pop());
        assert!(!abs_path.pop());
        assert_eq!(path, abs_path.as_path());
    }

    #[test]
    fn test_join() {
        let path = if cfg!(windows) {
            AbsoluteNormalizedPathBuf::try_from("c:\\foo\\bar".to_owned()).unwrap()
        } else {
            AbsoluteNormalizedPathBuf::try_from("/foo/bar".to_owned()).unwrap()
        };

        let path = path.join(ForwardRelativePath::new("baz").unwrap());
        assert_eq!(
            path.to_str().unwrap(),
            if cfg!(windows) {
                "c:\\foo\\bar\\baz"
            } else {
                "/foo/bar/baz"
            }
        );

        let path = path.join(ForwardRelativePath::empty());
        assert_eq!(
            path.to_str().unwrap(),
            if cfg!(windows) {
                "c:\\foo\\bar\\baz"
            } else {
                "/foo/bar/baz"
            }
        );
    }
}
