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
    hash::Hash,
    ops::Deref,
    path::{Path, PathBuf},
};

use derive_more::Display;
use gazebo::transmute;
use ref_cast::RefCast;
use relative_path::{RelativePath, RelativePathBuf};
use serde::Serialize;
use smallvec::SmallVec;
use thiserror::Error;

use crate::{
    absolute_forward_system_path::{AbsoluteForwardSystemPath, AbsoluteForwardSystemPathBuf},
    file_name::{FileName, FileNameBuf},
    fs_util,
};

/// A forward pointing, fully normalized relative path and owned pathbuf.
/// This means that there is no '.' or '..' in this path, and does not begin
/// with '/'.
///
/// This path is platform agnostic, so path separators are always '/'.
#[derive(Display, Debug, RefCast, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct RelativeForwardUnixPath(
    // Note we transmute between `ForwardRelativePath` and `str`.
    str,
);

/// The owned version of 'ForwardRelativePath', like how 'PathBuf' relates to
/// 'Path'
#[derive(Clone, Display, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct RelativeForwardUnixPathBuf(String);

impl AsRef<RelativePath> for RelativeForwardUnixPath {
    #[inline]
    fn as_ref(&self) -> &RelativePath {
        RelativePath::new(&self.0)
    }
}

impl AsRef<RelativePath> for RelativeForwardUnixPathBuf {
    #[inline]
    fn as_ref(&self) -> &RelativePath {
        RelativePath::new(&self.0)
    }
}

pub struct ForwardRelativePathIter<'a>(&'a RelativeForwardUnixPath);

impl<'a> Iterator for ForwardRelativePathIter<'a> {
    type Item = &'a FileName;

    #[inline]
    fn next(&mut self) -> Option<&'a FileName> {
        let (first, rem) = self.0.split_first()?;
        self.0 = rem;
        Some(first)
    }
}

impl<'a> Clone for ForwardRelativePathIter<'a> {
    fn clone(&self) -> Self {
        ForwardRelativePathIter(RelativeForwardUnixPath::unchecked_new(self.0.as_str()))
    }
}

impl RelativeForwardUnixPath {
    #[inline]
    pub fn unchecked_new<S: ?Sized + AsRef<str>>(s: &S) -> &Self {
        RelativeForwardUnixPath::ref_cast(s.as_ref())
    }

    #[inline]
    pub fn unchecked_new_box(s: Box<str>) -> Box<RelativeForwardUnixPath> {
        unsafe {
            // SAFETY: `ForwardRelativePath` is a transparent wrapper around `str`.
            transmute!(Box<str>, Box<RelativeForwardUnixPath>, s)
        }
    }

    #[inline]
    pub fn empty() -> &'static Self {
        RelativeForwardUnixPath::unchecked_new("")
    }

    /// Creates an 'ForwardRelativePath' if the given path represents a forward,
    /// normalized relative path, otherwise error.
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    /// use std::path::Path;
    ///
    /// assert!(RelativeForwardUnixPath::new("foo/bar").is_ok());
    /// assert!(RelativeForwardUnixPath::new("").is_ok());
    /// assert!(RelativeForwardUnixPath::new("./bar").is_err());
    /// assert!(RelativeForwardUnixPath::new("normalize/./bar").is_err());
    /// assert!(RelativeForwardUnixPath::new("/abs/bar").is_err());
    /// assert!(RelativeForwardUnixPath::new("foo//bar").is_err());
    /// assert!(RelativeForwardUnixPath::new("normalize/../bar").is_err());
    ///
    /// assert!(RelativeForwardUnixPath::new(Path::new("foo/bar")).is_ok());
    /// assert!(RelativeForwardUnixPath::new(Path::new("")).is_ok());
    /// assert!(RelativeForwardUnixPath::new(Path::new("./bar")).is_err());
    /// assert!(RelativeForwardUnixPath::new(Path::new("normalize/./bar")).is_err());
    /// assert!(RelativeForwardUnixPath::new(Path::new("/abs/bar")).is_err());
    /// assert!(RelativeForwardUnixPath::new(Path::new("normalize/../bar")).is_err());
    /// assert!(RelativeForwardUnixPath::new(Path::new("normalize\\bar")).is_err());
    /// assert!(RelativeForwardUnixPath::new(Path::new("normalize/bar/")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    // TODO(nga): this accepts `Path`, but should accept `str`
    //   because paths can have backslashes.
    //   Conversion of `Path` to `ForwardRelativePath` should be done via
    //   `ForwardRelativePathBuf` which should normalize slashes.
    #[inline]
    pub fn new<S: ?Sized + AsRef<Path>>(s: &S) -> anyhow::Result<&RelativeForwardUnixPath> {
        <&RelativeForwardUnixPath>::try_from(s.as_ref())
    }

    /// `ForwardRelativePath` requires no trailing slashes. This function
    /// constructs a path ignoring trailing slashes.
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// assert!(RelativeForwardUnixPath::new_trim_trailing_slashes("foo/bar").is_ok());
    /// assert!(RelativeForwardUnixPath::new_trim_trailing_slashes("foo/bar/").is_ok());
    /// assert!(RelativeForwardUnixPath::new_trim_trailing_slashes("foo/bar//").is_ok());
    /// assert!(RelativeForwardUnixPath::new_trim_trailing_slashes("foo//bar").is_err());
    /// ```
    pub fn new_trim_trailing_slashes<S: ?Sized + AsRef<Path>>(
        path: &S,
    ) -> anyhow::Result<&RelativeForwardUnixPath> {
        let path = path.as_ref();
        let path = path
            .to_str()
            .ok_or_else(|| ForwardRelativePathError::PathNotUtf8(path.display().to_string()))?;
        let path = path.trim_end_matches('/');
        RelativeForwardUnixPath::new(path)
    }

    /// Build an owned `AbsPathBuf` relative to `path` for the current relative
    /// path based on the supplied root.
    ///
    /// ```
    /// 
    /// use std::path::Path;
    /// use turborepo_paths::absolute_forward_system_path::{AbsoluteForwardSystemPath, AbsoluteForwardSystemPathBuf};
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// if cfg!(not(windows)) {
    ///     let path = RelativeForwardUnixPath::new("foo/bar")?.resolve(AbsoluteForwardSystemPath::new("/some")?);
    ///     assert_eq!(AbsoluteForwardSystemPathBuf::from("/some/foo/bar".into())?, path);
    /// } else {
    ///     let path = RelativeForwardUnixPath::new("foo/bar")?.resolve(AbsoluteForwardSystemPath::new("c:/some")?);
    ///     assert_eq!(AbsoluteForwardSystemPathBuf::from("c:/some/foo/bar".into())?, path);
    /// }
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    pub fn resolve<P: AsRef<AbsoluteForwardSystemPath>>(
        &self,
        relative_to: P,
    ) -> AbsoluteForwardSystemPathBuf {
        relative_to.as_ref().join(self)
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[inline]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Creates an owned 'ForwardRelativePathBuf' with path adjoined to self.
    ///
    /// ```
    /// use std::path::Path;
    /// use turborepo_paths::relative_forward_unix_path::{RelativeForwardUnixPathBuf, RelativeForwardUnixPath};
    ///
    /// let path = RelativeForwardUnixPath::new("foo/bar")?;
    /// let other = RelativeForwardUnixPath::new("baz")?;
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar/baz".to_owned()), path.join(other));
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    pub fn join<P: AsRef<RelativeForwardUnixPath>>(&self, path: P) -> RelativeForwardUnixPathBuf {
        let path = path.as_ref();
        if self.0.is_empty() {
            path.to_buf()
        } else if path.0.is_empty() {
            self.to_buf()
        } else {
            let mut buf = String::with_capacity(self.0.len() + 1 + path.0.len());
            buf.push_str(&self.0);
            buf.push('/');
            buf.push_str(&path.0);
            RelativeForwardUnixPathBuf::unchecked_new(buf)
        }
    }

    /// Returns a relative path of the parent directory
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// assert_eq!(
    ///     Some(RelativeForwardUnixPath::new("foo")?),
    ///     RelativeForwardUnixPath::new("foo/bar")?.parent()
    /// );
    /// assert_eq!(
    ///     Some(RelativeForwardUnixPath::new("")?),
    ///     RelativeForwardUnixPath::new("foo")?.parent()
    /// );
    /// assert_eq!(
    ///     None,
    ///     RelativeForwardUnixPath::new("")?.parent()
    /// );
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn parent(&self) -> Option<&RelativeForwardUnixPath> {
        let s = &self.0;
        for i in (0..s.len()).rev() {
            if s.as_bytes()[i] == b'/' {
                return Some(RelativeForwardUnixPath::unchecked_new(&s[..i]));
            }
        }
        if s.is_empty() {
            None
        } else {
            Some(RelativeForwardUnixPath::empty())
        }
    }

    /// Returns the final component of the `ForwardRelativePath`, if there is
    /// one.
    ///
    /// If the path is a normal file, this is the file name. If it's the path of
    /// a directory, this is the directory name.
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    /// use turborepo_paths::file_name::FileName;
    ///
    /// assert_eq!(Some(FileName::unchecked_new("ls")), RelativeForwardUnixPath::new("usr/bin/ls")?.file_name());
    /// assert_eq!(Some(FileName::unchecked_new("bin")), RelativeForwardUnixPath::new("usr/bin")?.file_name());
    /// assert_eq!(Some(FileName::unchecked_new("usr")), RelativeForwardUnixPath::new("usr")?.file_name());
    /// assert_eq!(None, RelativeForwardUnixPath::new("")?.file_name());
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn file_name(&self) -> Option<&FileName> {
        let s = &self.0;
        for (i, b) in s.bytes().enumerate().rev() {
            if b == b'/' {
                return Some(FileName::unchecked_new(&s[i + 1..]));
            }
        }
        if s.is_empty() {
            None
        } else {
            Some(FileName::unchecked_new(s))
        }
    }

    /// Get the first component of the path and the remaining path,
    /// of `None` if the path is empty.
    pub fn split_first(&self) -> Option<(&FileName, &RelativeForwardUnixPath)> {
        let s = &self.0;
        for (i, b) in s.bytes().enumerate() {
            if b == b'/' {
                return Some((
                    FileName::unchecked_new(&s[..i]),
                    RelativeForwardUnixPath::unchecked_new(&s[i + 1..]),
                ));
            }
        }
        if s.is_empty() {
            None
        } else {
            Some((FileName::unchecked_new(s), RelativeForwardUnixPath::empty()))
        }
    }

    /// Returns a 'ForwardRelativePath' that, when joined onto `base`, yields
    /// `self`.
    ///
    /// Error if `base` is not a prefix of `self` or the returned
    /// path is not a 'ForwardRelativePath'
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// let path = RelativeForwardUnixPath::new("test/haha/foo.txt")?;
    ///
    /// assert_eq!(
    ///     path.strip_prefix(RelativeForwardUnixPath::new("test/haha/foo.txt")?)?,
    ///     RelativeForwardUnixPath::new("")?
    /// );
    /// assert_eq!(
    ///     path.strip_prefix(RelativeForwardUnixPath::new("test/haha")?)?,
    ///     RelativeForwardUnixPath::new("foo.txt")?
    /// );
    /// assert_eq!(
    ///     path.strip_prefix(RelativeForwardUnixPath::new("test")?)?,
    ///     RelativeForwardUnixPath::new("haha/foo.txt")?
    /// );
    /// assert_eq!(
    ///     path.strip_prefix(RelativeForwardUnixPath::new("")?)?,
    ///     RelativeForwardUnixPath::new("test/haha/foo.txt")?
    /// );
    /// assert_eq!(path.strip_prefix(RelativeForwardUnixPath::new("asdf")?).is_err(), true);
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn strip_prefix<P: AsRef<RelativeForwardUnixPath>>(
        &self,
        base: P,
    ) -> anyhow::Result<&RelativeForwardUnixPath> {
        let base = base.as_ref();
        if base.0.is_empty() {
            Ok(self)
        } else if self.starts_with(base) {
            if self.0.len() == base.0.len() {
                Ok(RelativeForwardUnixPath::empty())
            } else {
                Ok(RelativeForwardUnixPath::unchecked_new(
                    &self.0[base.0.len() + 1..],
                ))
            }
        } else {
            Err(StripPrefixError(base.as_str().to_owned(), self.0.to_owned()).into())
        }
    }

    /// Determines whether `base` is a prefix of `self`.
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// let path = RelativeForwardUnixPath::new("some/foo")?;
    ///
    /// assert!(path.starts_with(RelativeForwardUnixPath::new("some/foo")?));
    /// assert!(path.starts_with(RelativeForwardUnixPath::new("some")?));
    /// assert!(!path.starts_with(RelativeForwardUnixPath::new("som")?));
    /// assert!(path.starts_with(RelativeForwardUnixPath::new("")?));
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn starts_with<P: AsRef<RelativeForwardUnixPath>>(&self, base: P) -> bool {
        let base = base.as_ref();
        base.0.is_empty()
            || (self.0.starts_with(&base.0)
                && (self.0.len() == base.0.len() || self.0.as_bytes()[base.0.len()] == b'/'))
    }

    /// Determines whether `child` is a suffix of `self`.
    /// Only considers whole path components to match.
    ///
    /// ```
    /// use std::path::Path;
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// let path = RelativeForwardUnixPath::new("some/foo")?;
    ///
    /// assert!(path.ends_with(RelativeForwardUnixPath::new("some/foo")?));
    /// assert!(path.ends_with(RelativeForwardUnixPath::new("foo")?));
    /// assert!(!path.ends_with(RelativeForwardUnixPath::new("oo")?));
    /// assert!(path.ends_with(RelativeForwardUnixPath::new("")?));
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn ends_with<P: AsRef<RelativeForwardUnixPath>>(&self, child: P) -> bool {
        let child = child.as_ref();
        child.0.is_empty()
            || (self.0.ends_with(&child.0)
                && (self.0.len() == child.0.len()
                    || self.0.as_bytes()[self.0.len() - child.0.len() - 1] == b'/'))
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
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// let path = RelativeForwardUnixPath::new("foo.rs")?;
    ///
    /// assert_eq!(Some("foo"), path.file_stem());
    /// assert_eq!(Some("foo.bar"), RelativeForwardUnixPath::new("hi/foo.bar.rs")?.file_stem());
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn file_stem(&self) -> Option<&str> {
        let file = self.file_name();
        file.map(|f| {
            let f = f.as_str();
            for (i, b) in f.bytes().enumerate().rev() {
                if b == b'.' && i > 0 {
                    return &f[0..i];
                }
            }

            f
        })
    }

    /// Extracts the extension of [`self.file_name`], if possible.
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// assert_eq!(Some("rs"), RelativeForwardUnixPath::new("hi/foo.rs")?.extension());
    /// assert_eq!(Some("rs"), RelativeForwardUnixPath::new("hi/foo.bar.rs")?.extension());
    /// assert_eq!(None, RelativeForwardUnixPath::new(".git")?.extension());
    /// assert_eq!(None, RelativeForwardUnixPath::new("foo/.git")?.extension());
    /// assert_eq!(None, RelativeForwardUnixPath::new("")?.extension());
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn extension(&self) -> Option<&str> {
        let s = &self.0;
        let bytes = s.as_bytes();
        let mut i = s.len().checked_sub(1)?;
        while i > 0 {
            let b = bytes[i];
            if b == b'/' {
                return None;
            }
            if b == b'.' {
                if bytes[i - 1] == b'/' {
                    return None;
                }
                return Some(&s[i + 1..]);
            }

            i -= 1;
        }
        None
    }

    /// Build an owned `ForwardRelativePathBuf`, joined with the given path and
    /// normalized.
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::{RelativeForwardUnixPath, RelativeForwardUnixPathBuf};
    ///
    /// assert_eq!(
    ///     RelativeForwardUnixPathBuf::unchecked_new("foo/baz.txt".into()),
    ///     RelativeForwardUnixPath::new("foo/bar")?.join_normalized("../baz.txt")?,
    /// );
    ///
    /// assert_eq!(
    ///     RelativeForwardUnixPath::new("foo")?.join_normalized("../../baz.txt").is_err(),
    ///     true
    /// );
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn join_normalized<P: AsRef<RelativePath>>(
        &self,
        path: P,
    ) -> anyhow::Result<RelativeForwardUnixPathBuf> {
        let self_rel_path: &RelativePath = self.as_ref();
        let inner = self_rel_path.join_normalized(path.as_ref());
        RelativeForwardUnixPathBuf::try_from(inner)
    }

    /// Append a relative system path, obtained frome e.g. `read_link`.
    ///
    /// The path will be converted to an internal path (i.e. forward slashes)
    /// before joining.
    pub fn join_system(&self, path: &Path) -> anyhow::Result<RelativeForwardUnixPathBuf> {
        let path = fs_util::relative_path_from_system(path)?;
        self.join_normalized(path)
    }

    /// Iterator over the components of this path
    ///
    /// ```
    /// use turborepo_paths::file_name::FileName;
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// let p = RelativeForwardUnixPath::new("foo/bar/baz")?;
    /// let mut it = p.iter();
    ///
    /// assert_eq!(
    ///     it.next(),
    ///     Some(FileName::unchecked_new("foo"))
    /// );
    /// assert_eq!(
    ///     it.next(),
    ///     Some(FileName::unchecked_new("bar"))
    /// );
    /// assert_eq!(
    ///     it.next(),
    ///     Some(FileName::unchecked_new("baz"))
    /// );
    /// assert_eq!(
    ///     it.next(),
    ///     None
    /// );
    /// assert_eq!(
    ///     it.next(),
    ///     None
    /// );
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    pub fn iter(&self) -> ForwardRelativePathIter<'_> {
        ForwardRelativePathIter(self)
    }

    /// Strip a given number of components from the prefix of a path,
    /// returning the remaining path or `None` if there were none left.
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    ///
    /// let p = RelativeForwardUnixPath::new("foo/bar/baz")?;
    /// assert_eq!(
    ///     p.strip_prefix_components(0),
    ///     Some(RelativeForwardUnixPath::new("foo/bar/baz")?),
    /// );
    /// assert_eq!(
    ///     p.strip_prefix_components(1),
    ///     Some(RelativeForwardUnixPath::new("bar/baz")?),
    /// );
    /// assert_eq!(
    ///     p.strip_prefix_components(2),
    ///     Some(RelativeForwardUnixPath::new("baz")?),
    /// );
    /// assert_eq!(
    ///     p.strip_prefix_components(3),
    ///     Some(RelativeForwardUnixPath::new("")?),
    /// );
    /// assert_eq!(
    ///     p.strip_prefix_components(4),
    ///     None,
    /// );
    /// # anyhow::Ok(())
    /// ```
    pub fn strip_prefix_components(&self, components: usize) -> Option<&Self> {
        let mut rem = self;
        for _ in 0..components {
            rem = rem.split_first()?.1;
        }
        Some(rem)
    }

    #[inline]
    pub fn to_buf(&self) -> RelativeForwardUnixPathBuf {
        self.to_owned()
    }

    pub fn to_box(&self) -> Box<RelativeForwardUnixPath> {
        self.to_buf().into_box()
    }

    /// Return a RelativePath represenation of this ForwardRelativePath.
    #[inline]
    pub fn as_relative_path(&self) -> &RelativePath {
        RelativePath::new(&self.0)
    }
}

impl RelativeForwardUnixPathBuf {
    #[inline]
    pub fn new(s: String) -> anyhow::Result<RelativeForwardUnixPathBuf> {
        RelativeForwardUnixPath::new(&s)?;
        Ok(RelativeForwardUnixPathBuf(s))
    }

    #[inline]
    pub fn empty() -> Self {
        Self("".to_owned())
    }

    #[inline]
    pub fn unchecked_new(s: String) -> Self {
        Self(s)
    }

    /// Creates a new 'ForwardRelativePathBuf' with a given capacity used to
    /// create the internal 'String'. See 'with_capacity' defined on
    /// 'String'
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self(String::with_capacity(cap))
    }

    /// Returns the capacity of the underlying 'String'
    #[inline]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    /// Invokes 'reserve' on the underlying 'String'
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional)
    }

    /// Invokes 'shrink_to_fit' on the underlying 'String'
    #[inline]
    pub fn shrink_to_fit(&mut self) {
        self.0.shrink_to_fit()
    }

    /// Invokes 'shrink_to' on the underlying 'String'
    #[inline]
    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.0.shrink_to(min_capacity)
    }

    /// Pushes a `ForwardRelativePath` to the existing buffer
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::{RelativeForwardUnixPath, RelativeForwardUnixPathBuf};
    ///
    /// let mut path = RelativeForwardUnixPathBuf::unchecked_new("foo".to_owned());
    /// path.push(RelativeForwardUnixPath::unchecked_new("bar"));
    ///
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar".to_owned()), path);
    ///
    /// path.push(RelativeForwardUnixPath::unchecked_new("more/file.rs"));
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar/more/file.rs".to_owned()), path);
    ///
    /// path.push(RelativeForwardUnixPath::empty());
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar/more/file.rs".to_owned()), path);
    ///
    /// let mut path = RelativeForwardUnixPathBuf::unchecked_new("".to_owned());
    /// path.push(RelativeForwardUnixPath::unchecked_new("foo"));
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo".to_owned()), path);
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn push<P: AsRef<RelativeForwardUnixPath>>(&mut self, path: P) {
        if path.as_ref().0.is_empty() {
            return;
        }

        if !self.0.is_empty() {
            self.0.push('/');
        }
        self.0.push_str(path.as_ref().as_str())
    }

    pub fn concat<'a, I: IntoIterator<Item = &'a RelativeForwardUnixPath> + Copy>(
        items: I,
    ) -> RelativeForwardUnixPathBuf {
        let mut cap = 0;
        for item in items {
            if !item.is_empty() {
                if cap != 0 {
                    // `/`.
                    cap += 1;
                }
                cap += item.0.len();
            }
        }
        let mut path = RelativeForwardUnixPathBuf::with_capacity(cap);
        for item in items {
            path.push(item);
        }
        // Cheap self-test.
        assert!(path.0.len() == cap);
        path
    }

    /// Pushes a `RelativePath` to the existing buffer, normalizing it.
    /// Note that this does not visit the filesystem to resolve `..`s. Instead,
    /// it cancels out the components directly, similar to
    /// `join_normalized`.
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPathBuf;
    /// use turborepo_paths::RelativePath;
    ///
    /// let mut path = RelativeForwardUnixPathBuf::unchecked_new("foo".to_owned());
    /// path.push_normalized(RelativePath::new("bar"))?;
    ///
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new("more/file.rs"))?;
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar/more/file.rs".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new("../other.rs"))?;
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar/more/other.rs".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new(".."))?;
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo/bar/more".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new("../.."))?;
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("foo".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new(".."))?;
    /// assert_eq!(RelativeForwardUnixPathBuf::unchecked_new("".to_owned()), path);
    ///
    /// assert!(path.push_normalized(RelativePath::new("..")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn push_normalized<P: AsRef<RelativePath>>(&mut self, path: P) -> anyhow::Result<()> {
        let buf = &mut self.0;
        let mut insert_idx = buf.len();
        let bytes = path.as_ref().as_str().as_bytes();

        let mut i = 0;
        let mut j = i;
        while j < bytes.len() {
            assert!(i <= bytes.len());
            j = i;
            while j != bytes.len() {
                if bytes[j] == b'/' {
                    break;
                }
                j += 1;
            }
            if i == j {
                // Double slashes or trailing slash.
                unreachable!("not a relative path");
            } else if j == i + 1 && bytes[i] == b'.' {
                // Current directory. Skip this segment and do nothing
            } else if j == i + 2 && bytes[i] == b'.' && bytes[i + 1] == b'.' {
                // Parent directory. Move the insert index

                if insert_idx == 0 {
                    // if we are already at 0, then we cannot move towards the parent without
                    // having this path still be forward pointing
                    return Err(anyhow::anyhow!(
                        ForwardRelativePathError::RelativizationError(path.as_ref().to_string())
                    ));
                }

                let mut buf_i = insert_idx;
                let buf_bytes = buf.as_bytes();
                // note we don't bother checking when buf_i is 0, because that would imply our
                // current forward relative path starts with '/', which would imply that it's
                // not relative, which is unreachable code.
                while buf_i > 0 {
                    buf_i -= 1;

                    if buf_bytes[buf_i] == b'/' {
                        break;
                    }
                }
                // we got here because we either found a '/', or we got to the beginning of the
                // current path, but starting with something in it, which means that we are now
                // at the beginning segment, so insert_idx can be the beginning
                insert_idx = buf_i;
            } else {
                // just a path segment to add

                // first add the '/' since our path representation does not have ending slashes
                if insert_idx < buf.len() {
                    buf.replace_range(insert_idx..=insert_idx, "/");
                } else {
                    buf.push('/');
                }

                let seg_to_add = unsafe {
                    // safe because this is the buf from a `RelativePath`, which enforces `utf8`

                    // also `j` rather than `j+1` to exclude the ending `/`,
                    // or not run out of bounds if `j = bytes.len()`
                    std::str::from_utf8_unchecked(&bytes[i..j])
                };
                if insert_idx + 1 < buf.len() {
                    buf.replace_range(insert_idx + 1.., seg_to_add);
                } else {
                    buf.push_str(seg_to_add);
                }

                insert_idx = buf.len();
            }
            i = j + 1;
        }

        if insert_idx < buf.len() {
            buf.replace_range(insert_idx.., "");
        }

        Ok(())
    }

    #[inline]
    pub fn into_string(self) -> String {
        self.0
    }

    pub fn into_box(self) -> Box<RelativeForwardUnixPath> {
        let s: Box<str> = self.0.into_boxed_str();
        RelativeForwardUnixPath::unchecked_new_box(s)
    }
}

/// Errors from ForwardRelativePath creation
#[derive(Error, Debug)]
enum ForwardRelativePathError {
    #[error("expected a relative path but got an absolute path instead: `{0}`")]
    PathNotRelative(String),
    #[error("expected a normalized path but got an un-normalized path instead: `{0}`")]
    PathNotNormalized(String),
    #[error("Path is not UTF-8: `{0}`")]
    PathNotUtf8(String),
    #[error("relativizing path `{0}` results would result in a non-forward relative path")]
    RelativizationError(String),
}

/// Error from 'strip_prefix'
#[derive(Error, Debug)]
#[error("`{0}` is not a base of `{1}`")]
pub struct StripPrefixError(String, String);

impl<'a> IntoIterator for &'a RelativeForwardUnixPath {
    type Item = &'a FileName;
    type IntoIter = ForwardRelativePathIter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> TryFrom<&'a str> for &'a RelativeForwardUnixPath {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    /// use std::convert::TryFrom;
    ///
    /// assert!(<&RelativeForwardUnixPath>::try_from("foo/bar").is_ok());
    /// assert!(<&RelativeForwardUnixPath>::try_from("").is_ok());
    /// assert!(<&RelativeForwardUnixPath>::try_from("./bar").is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from("normalize/./bar").is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from("/abs/bar").is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from("normalize/../bar").is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(s: &'a str) -> anyhow::Result<&'a RelativeForwardUnixPath> {
        ForwardRelativePathVerifier::verify_str(s)?;
        Ok(RelativeForwardUnixPath::ref_cast(s))
    }
}

impl<'a> From<&'a FileName> for &'a RelativeForwardUnixPath {
    #[inline]
    fn from(p: &'a FileName) -> Self {
        RelativeForwardUnixPath::unchecked_new(p.as_str())
    }
}

impl<'a> TryFrom<&'a Path> for &'a RelativeForwardUnixPath {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    /// use std::convert::TryFrom;
    /// use std::path::Path;
    ///
    /// assert!(<&RelativeForwardUnixPath>::try_from(Path::new("foo/bar")).is_ok());
    /// assert!(<&RelativeForwardUnixPath>::try_from(Path::new("")).is_ok());
    /// assert!(<&RelativeForwardUnixPath>::try_from(Path::new("./bar")).is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from(Path::new("normalize/./bar")).is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from(Path::new("/abs/bar")).is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from(Path::new("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    fn try_from(s: &'a Path) -> anyhow::Result<&'a RelativeForwardUnixPath> {
        let s = s
            .as_os_str()
            .to_str()
            .ok_or_else(|| ForwardRelativePathError::PathNotUtf8(s.display().to_string()))?;
        ForwardRelativePathVerifier::verify_str(s)?;
        Ok(RelativeForwardUnixPath::unchecked_new(s))
    }
}

impl<'a> TryFrom<&'a RelativePath> for &'a RelativeForwardUnixPath {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPath;
    /// use std::convert::TryFrom;
    /// use turborepo_paths::RelativePath;
    ///
    /// assert!(<&RelativeForwardUnixPath>::try_from(RelativePath::new("foo/bar")).is_ok());
    /// assert!(<&RelativeForwardUnixPath>::try_from(RelativePath::new("")).is_ok());
    /// assert!(<&RelativeForwardUnixPath>::try_from(RelativePath::new("./bar")).is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from(RelativePath::new("normalize/./bar")).is_err());
    /// assert!(<&RelativeForwardUnixPath>::try_from(RelativePath::new("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(p: &'a RelativePath) -> anyhow::Result<&'a RelativeForwardUnixPath> {
        ForwardRelativePathVerifier::verify_str(p.as_str())?;
        Ok(RelativeForwardUnixPath::unchecked_new(p.as_str()))
    }
}

impl From<RelativeForwardUnixPathBuf> for RelativePathBuf {
    fn from(p: RelativeForwardUnixPathBuf) -> Self {
        RelativePathBuf::from(p.0)
    }
}

impl TryFrom<String> for RelativeForwardUnixPathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPathBuf;
    /// use std::convert::TryFrom;
    ///
    /// assert!(RelativeForwardUnixPathBuf::try_from("foo/bar".to_owned()).is_ok());
    /// assert!(RelativeForwardUnixPathBuf::try_from("".to_owned()).is_ok());
    /// assert!(RelativeForwardUnixPathBuf::try_from("./bar".to_owned()).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from("normalize/./bar".to_owned()).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from("/abs/bar".to_owned()).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from("normalize/../bar".to_owned()).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(s: String) -> anyhow::Result<RelativeForwardUnixPathBuf> {
        ForwardRelativePathVerifier::verify_str(&s)?;
        Ok(RelativeForwardUnixPathBuf(s))
    }
}

impl TryFrom<PathBuf> for RelativeForwardUnixPathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPathBuf;
    /// use turborepo_paths::RelativePathBuf;
    /// use std::convert::TryFrom;
    /// use std::path::PathBuf;
    ///
    /// assert!(RelativeForwardUnixPathBuf::try_from(PathBuf::from("foo/bar")).is_ok());
    /// assert!(RelativeForwardUnixPathBuf::try_from(PathBuf::from("")).is_ok());
    /// assert!(RelativeForwardUnixPathBuf::try_from(PathBuf::from("./bar")).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from(PathBuf::from("normalize/./bar")).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from(PathBuf::from("/abs/bar")).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from(PathBuf::from("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    fn try_from(p: PathBuf) -> anyhow::Result<RelativeForwardUnixPathBuf> {
        // RelativePathBuf::from_path actually creates a copy.
        // avoid the copy by constructing RelativePathBuf from the underlying String
        RelativeForwardUnixPathBuf::try_from(p.into_os_string().into_string().map_err(|_| {
            relative_path::FromPathError::from(relative_path::FromPathErrorKind::NonUtf8)
        })?)
    }
}

impl TryFrom<RelativePathBuf> for RelativeForwardUnixPathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// use turborepo_paths::relative_forward_unix_path::RelativeForwardUnixPathBuf;
    /// use turborepo_paths::RelativePathBuf;
    /// use std::convert::TryFrom;
    ///
    /// assert!(RelativeForwardUnixPathBuf::try_from(RelativePathBuf::from("foo/bar")).is_ok());
    /// assert!(RelativeForwardUnixPathBuf::try_from(RelativePathBuf::from("")).is_ok());
    /// assert!(RelativeForwardUnixPathBuf::try_from(RelativePathBuf::from("./bar")).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from(RelativePathBuf::from("normalize/./bar")).is_err());
    /// assert!(RelativeForwardUnixPathBuf::try_from(RelativePathBuf::from("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(p: RelativePathBuf) -> anyhow::Result<RelativeForwardUnixPathBuf> {
        RelativeForwardUnixPathBuf::try_from(p.into_string())
    }
}

impl ToOwned for RelativeForwardUnixPath {
    type Owned = RelativeForwardUnixPathBuf;

    #[inline]
    fn to_owned(&self) -> RelativeForwardUnixPathBuf {
        RelativeForwardUnixPathBuf::unchecked_new(self.0.to_owned())
    }
}

impl AsRef<RelativeForwardUnixPath> for RelativeForwardUnixPath {
    #[inline]
    fn as_ref(&self) -> &RelativeForwardUnixPath {
        self
    }
}

impl AsRef<RelativeForwardUnixPath> for RelativeForwardUnixPathBuf {
    #[inline]
    fn as_ref(&self) -> &RelativeForwardUnixPath {
        RelativeForwardUnixPath::unchecked_new(&self.0)
    }
}

impl Borrow<RelativeForwardUnixPath> for RelativeForwardUnixPathBuf {
    #[inline]
    fn borrow(&self) -> &RelativeForwardUnixPath {
        self.as_ref()
    }
}

impl Deref for RelativeForwardUnixPathBuf {
    type Target = RelativeForwardUnixPath;

    #[inline]
    fn deref(&self) -> &RelativeForwardUnixPath {
        RelativeForwardUnixPath::unchecked_new(&self.0)
    }
}

/// Normalize ForwardRelativePath path if needed.
pub struct ForwardRelativePathNormalizer {}

impl ForwardRelativePathNormalizer {
    pub fn normalize_path<P: AsRef<Path> + ?Sized>(
        rel_path: &P,
    ) -> anyhow::Result<Cow<RelativeForwardUnixPath>> {
        let rel_path = rel_path.as_ref();
        if !rel_path.is_relative() {
            return Err(anyhow::anyhow!(ForwardRelativePathError::PathNotRelative(
                rel_path.display().to_string(),
            )));
        }
        let path_str = rel_path
            .to_str()
            .ok_or_else(|| ForwardRelativePathError::PathNotUtf8(rel_path.display().to_string()))?;
        let bytes = path_str.as_bytes();
        if cfg!(windows) && memchr::memchr(b'\\', bytes).is_some() {
            let normalized_path = path_str.replace('\\', "/");
            Ok(Cow::Owned(RelativeForwardUnixPathBuf::try_from(
                normalized_path,
            )?))
        } else {
            Ok(Cow::Borrowed(RelativeForwardUnixPath::new(path_str)?))
        }
    }
}

/// Verifier for ForwardRelativePath to ensure the path is fully relative, and
/// normalized
struct ForwardRelativePathVerifier {}

impl ForwardRelativePathVerifier {
    fn verify_str(rel_path: &str) -> anyhow::Result<()> {
        #[cold]
        #[inline(never)]
        fn err(rel_path: &str) -> anyhow::Error {
            anyhow::anyhow!(ForwardRelativePathError::PathNotNormalized(
                rel_path.to_owned()
            ))
        }

        let bytes = rel_path.as_bytes();
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes[0] == b'/' {
            return Err(anyhow::anyhow!(ForwardRelativePathError::PathNotRelative(
                rel_path.to_owned()
            )));
        }

        if memchr::memchr(b'\\', bytes).is_some() {
            return Err(err(rel_path));
        }

        let mut i = 0;
        loop {
            assert!(i <= bytes.len());
            let mut j = i;
            while j != bytes.len() {
                if bytes[j] == b'/' {
                    break;
                }
                j += 1;
            }
            if i == j {
                // Double slashes or trailing slash.
                return Err(err(rel_path));
            }
            if j == i + 1 && bytes[i] == b'.' {
                // Current directory.
                return Err(err(rel_path));
            }
            if j == i + 2 && bytes[i] == b'.' && bytes[i + 1] == b'.' {
                // Parent directory.
                return Err(err(rel_path));
            }
            if j == bytes.len() {
                return Ok(());
            }
            i = j + 1;
        }
    }
}

impl<'a> FromIterator<&'a FileName> for Option<RelativeForwardUnixPathBuf> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = &'a FileName>,
    {
        from_iter::<20, _>(iter)
    }
}

impl<'a> FromIterator<&'a FileNameBuf> for Option<RelativeForwardUnixPathBuf> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = &'a FileNameBuf>,
    {
        iter.into_iter()
            .map(<FileNameBuf as AsRef<FileName>>::as_ref)
            .collect()
    }
}

fn from_iter<'a, const N: usize, I>(iter: I) -> Option<RelativeForwardUnixPathBuf>
where
    I: IntoIterator<Item = &'a FileName>,
{
    // Collect up to 20 pointers to the stack. This avoids a reallocation when
    // joining paths of up to 20 components.
    let parts = iter.into_iter().collect::<SmallVec<[_; 20]>>();

    let mut first = true;
    let mut size = 0;
    for part in &parts {
        if !first {
            size += 1; // For `/`
        }
        size += part.as_str().len();
        first = false;
    }

    let mut ret = String::with_capacity(size);
    for part in &parts {
        if !ret.is_empty() {
            ret.push('/');
        }
        ret.push_str(part.as_ref());
    }

    if ret.is_empty() {
        None
    } else {
        Some(RelativeForwardUnixPathBuf(ret))
    }
}

#[cfg(test)]
mod tests {
    use crate::relative_forward_unix_path::{
        from_iter, FileName, RelativeForwardUnixPath, RelativeForwardUnixPathBuf,
    };

    #[test]
    fn forward_path_is_comparable() -> anyhow::Result<()> {
        let path1_buf = RelativeForwardUnixPathBuf::unchecked_new("foo".into());
        let path2_buf = RelativeForwardUnixPathBuf::unchecked_new("foo".into());
        let path3_buf = RelativeForwardUnixPathBuf::unchecked_new("bar".into());

        let path1 = RelativeForwardUnixPath::new("foo")?;
        let path2 = RelativeForwardUnixPath::new("foo")?;
        let path3 = RelativeForwardUnixPath::new("bar")?;

        let str2 = "foo";
        let str3 = "bar";
        let str_abs = "/ble";

        let string2 = "foo".to_owned();
        let string3 = "bar".to_owned();
        let string_abs = "/ble".to_owned();

        assert_eq!(path1_buf, path2_buf);
        assert_ne!(path1_buf, path3_buf);

        assert_eq!(path1, path2);
        assert_ne!(path1, path3);

        assert_eq!(path1_buf, path2);
        assert_ne!(path1, path3_buf);

        assert_eq!(path1_buf, str2);
        assert_ne!(path1_buf, str3);
        assert_ne!(path1_buf, str_abs);

        assert_eq!(path1, str2);
        assert_ne!(path1, str3);
        assert_ne!(path1, str_abs);

        assert_eq!(path1_buf, string2);
        assert_ne!(path1_buf, string3);
        assert_ne!(path1_buf, string_abs);

        assert_eq!(path1, string2);
        assert_ne!(path1, string3);
        assert_ne!(path1, string_abs);

        Ok(())
    }

    #[test]
    fn test_concat() {
        assert_eq!(
            RelativeForwardUnixPath::new("").unwrap(),
            AsRef::<RelativeForwardUnixPath>::as_ref(&RelativeForwardUnixPathBuf::concat([]))
        );
        assert_eq!(
            RelativeForwardUnixPath::new("foo/bar/baz").unwrap(),
            AsRef::<RelativeForwardUnixPath>::as_ref(&RelativeForwardUnixPathBuf::concat([
                RelativeForwardUnixPath::new("foo").unwrap(),
                RelativeForwardUnixPath::new("bar/baz").unwrap(),
            ]))
        );
        assert_eq!(
            RelativeForwardUnixPath::new("foo/bar/baz").unwrap(),
            AsRef::<RelativeForwardUnixPath>::as_ref(&RelativeForwardUnixPathBuf::concat([
                RelativeForwardUnixPath::new("").unwrap(),
                RelativeForwardUnixPath::new("foo").unwrap(),
                RelativeForwardUnixPath::new("bar/baz").unwrap(),
            ]))
        );
    }

    #[test]
    fn test_from_iter() {
        let parts = &["foo", "bar", "baz"]
            .into_iter()
            .map(FileName::unchecked_new)
            .collect::<Vec<_>>();

        let expected = Some(RelativeForwardUnixPath::unchecked_new("foo/bar/baz").to_buf());

        assert_eq!(from_iter::<1, _>(parts.iter().copied()), expected);
        assert_eq!(from_iter::<2, _>(parts.iter().copied()), expected);
        assert_eq!(from_iter::<3, _>(parts.iter().copied()), expected);
        assert_eq!(from_iter::<4, _>(parts.iter().copied()), expected);
    }
}
