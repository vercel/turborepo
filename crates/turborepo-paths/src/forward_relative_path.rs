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
    absolute_normalized_path::{AbsoluteNormalizedPath, AbsoluteNormalizedPathBuf},
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
pub struct ForwardRelativePath(
    // Note we transmute between `ForwardRelativePath` and `str`.
    str,
);

/// The owned version of 'ForwardRelativePath', like how 'PathBuf' relates to
/// 'Path'
#[derive(Clone, Display, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct ForwardRelativePathBuf(String);

impl AsRef<RelativePath> for ForwardRelativePath {
    #[inline]
    fn as_ref(&self) -> &RelativePath {
        RelativePath::new(&self.0)
    }
}

impl AsRef<RelativePath> for ForwardRelativePathBuf {
    #[inline]
    fn as_ref(&self) -> &RelativePath {
        RelativePath::new(&self.0)
    }
}

pub struct ForwardRelativePathIter<'a>(&'a ForwardRelativePath);

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
        ForwardRelativePathIter(ForwardRelativePath::unchecked_new(self.0.as_str()))
    }
}

impl ForwardRelativePath {
    #[inline]
    pub fn unchecked_new<S: ?Sized + AsRef<str>>(s: &S) -> &Self {
        ForwardRelativePath::ref_cast(s.as_ref())
    }

    #[inline]
    pub fn unchecked_new_box(s: Box<str>) -> Box<ForwardRelativePath> {
        unsafe {
            // SAFETY: `ForwardRelativePath` is a transparent wrapper around `str`.
            transmute!(Box<str>, Box<ForwardRelativePath>, s)
        }
    }

    #[inline]
    pub fn empty() -> &'static Self {
        ForwardRelativePath::unchecked_new("")
    }

    /// Creates an 'ForwardRelativePath' if the given path represents a forward,
    /// normalized relative path, otherwise error.
    ///
    /// ```
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    /// use std::path::Path;
    ///
    /// assert!(ForwardRelativePath::new("foo/bar").is_ok());
    /// assert!(ForwardRelativePath::new("").is_ok());
    /// assert!(ForwardRelativePath::new("./bar").is_err());
    /// assert!(ForwardRelativePath::new("normalize/./bar").is_err());
    /// assert!(ForwardRelativePath::new("/abs/bar").is_err());
    /// assert!(ForwardRelativePath::new("foo//bar").is_err());
    /// assert!(ForwardRelativePath::new("normalize/../bar").is_err());
    ///
    /// assert!(ForwardRelativePath::new(Path::new("foo/bar")).is_ok());
    /// assert!(ForwardRelativePath::new(Path::new("")).is_ok());
    /// assert!(ForwardRelativePath::new(Path::new("./bar")).is_err());
    /// assert!(ForwardRelativePath::new(Path::new("normalize/./bar")).is_err());
    /// assert!(ForwardRelativePath::new(Path::new("/abs/bar")).is_err());
    /// assert!(ForwardRelativePath::new(Path::new("normalize/../bar")).is_err());
    /// assert!(ForwardRelativePath::new(Path::new("normalize\\bar")).is_err());
    /// assert!(ForwardRelativePath::new(Path::new("normalize/bar/")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    // TODO(nga): this accepts `Path`, but should accept `str`
    //   because paths can have backslashes.
    //   Conversion of `Path` to `ForwardRelativePath` should be done via
    //   `ForwardRelativePathBuf` which should normalize slashes.
    #[inline]
    pub fn new<S: ?Sized + AsRef<Path>>(s: &S) -> anyhow::Result<&ForwardRelativePath> {
        <&ForwardRelativePath>::try_from(s.as_ref())
    }

    /// `ForwardRelativePath` requires no trailing slashes. This function
    /// constructs a path ignoring trailing slashes.
    ///
    /// ```
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// assert!(ForwardRelativePath::new_trim_trailing_slashes("foo/bar").is_ok());
    /// assert!(ForwardRelativePath::new_trim_trailing_slashes("foo/bar/").is_ok());
    /// assert!(ForwardRelativePath::new_trim_trailing_slashes("foo/bar//").is_ok());
    /// assert!(ForwardRelativePath::new_trim_trailing_slashes("foo//bar").is_err());
    /// ```
    pub fn new_trim_trailing_slashes<S: ?Sized + AsRef<Path>>(
        path: &S,
    ) -> anyhow::Result<&ForwardRelativePath> {
        let path = path.as_ref();
        let path = path
            .to_str()
            .ok_or_else(|| ForwardRelativePathError::PathNotUtf8(path.display().to_string()))?;
        let path = path.trim_end_matches('/');
        ForwardRelativePath::new(path)
    }

    /// Build an owned `AbsPathBuf` relative to `path` for the current relative
    /// path based on the supplied root.
    ///
    /// ```
    /// 
    /// use std::path::Path;
    /// use turborepo_paths::absolute_normalized_path::{AbsoluteNormalizedPath, AbsoluteNormalizedPathBuf};
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// if cfg!(not(windows)) {
    ///     let path = ForwardRelativePath::new("foo/bar")?.resolve(AbsoluteNormalizedPath::new("/some")?);
    ///     assert_eq!(AbsoluteNormalizedPathBuf::from("/some/foo/bar".into())?, path);
    /// } else {
    ///     let path = ForwardRelativePath::new("foo/bar")?.resolve(AbsoluteNormalizedPath::new("c:/some")?);
    ///     assert_eq!(AbsoluteNormalizedPathBuf::from("c:/some/foo/bar".into())?, path);
    /// }
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    pub fn resolve<P: AsRef<AbsoluteNormalizedPath>>(
        &self,
        relative_to: P,
    ) -> AbsoluteNormalizedPathBuf {
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
    /// use turborepo_paths::forward_relative_path::{ForwardRelativePathBuf, ForwardRelativePath};
    ///
    /// let path = ForwardRelativePath::new("foo/bar")?;
    /// let other = ForwardRelativePath::new("baz")?;
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar/baz".to_owned()), path.join(other));
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    pub fn join<P: AsRef<ForwardRelativePath>>(&self, path: P) -> ForwardRelativePathBuf {
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
            ForwardRelativePathBuf::unchecked_new(buf)
        }
    }

    /// Returns a relative path of the parent directory
    ///
    /// ```
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// assert_eq!(
    ///     Some(ForwardRelativePath::new("foo")?),
    ///     ForwardRelativePath::new("foo/bar")?.parent()
    /// );
    /// assert_eq!(
    ///     Some(ForwardRelativePath::new("")?),
    ///     ForwardRelativePath::new("foo")?.parent()
    /// );
    /// assert_eq!(
    ///     None,
    ///     ForwardRelativePath::new("")?.parent()
    /// );
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn parent(&self) -> Option<&ForwardRelativePath> {
        let s = &self.0;
        for i in (0..s.len()).rev() {
            if s.as_bytes()[i] == b'/' {
                return Some(ForwardRelativePath::unchecked_new(&s[..i]));
            }
        }
        if s.is_empty() {
            None
        } else {
            Some(ForwardRelativePath::empty())
        }
    }

    /// Returns the final component of the `ForwardRelativePath`, if there is
    /// one.
    ///
    /// If the path is a normal file, this is the file name. If it's the path of
    /// a directory, this is the directory name.
    ///
    /// ```
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    /// use turborepo_paths::file_name::FileName;
    ///
    /// assert_eq!(Some(FileName::unchecked_new("ls")), ForwardRelativePath::new("usr/bin/ls")?.file_name());
    /// assert_eq!(Some(FileName::unchecked_new("bin")), ForwardRelativePath::new("usr/bin")?.file_name());
    /// assert_eq!(Some(FileName::unchecked_new("usr")), ForwardRelativePath::new("usr")?.file_name());
    /// assert_eq!(None, ForwardRelativePath::new("")?.file_name());
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
    pub fn split_first(&self) -> Option<(&FileName, &ForwardRelativePath)> {
        let s = &self.0;
        for (i, b) in s.bytes().enumerate() {
            if b == b'/' {
                return Some((
                    FileName::unchecked_new(&s[..i]),
                    ForwardRelativePath::unchecked_new(&s[i + 1..]),
                ));
            }
        }
        if s.is_empty() {
            None
        } else {
            Some((FileName::unchecked_new(s), ForwardRelativePath::empty()))
        }
    }

    /// Returns a 'ForwardRelativePath' that, when joined onto `base`, yields
    /// `self`.
    ///
    /// Error if `base` is not a prefix of `self` or the returned
    /// path is not a 'ForwardRelativePath'
    ///
    /// ```
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// let path = ForwardRelativePath::new("test/haha/foo.txt")?;
    ///
    /// assert_eq!(
    ///     path.strip_prefix(ForwardRelativePath::new("test/haha/foo.txt")?)?,
    ///     ForwardRelativePath::new("")?
    /// );
    /// assert_eq!(
    ///     path.strip_prefix(ForwardRelativePath::new("test/haha")?)?,
    ///     ForwardRelativePath::new("foo.txt")?
    /// );
    /// assert_eq!(
    ///     path.strip_prefix(ForwardRelativePath::new("test")?)?,
    ///     ForwardRelativePath::new("haha/foo.txt")?
    /// );
    /// assert_eq!(
    ///     path.strip_prefix(ForwardRelativePath::new("")?)?,
    ///     ForwardRelativePath::new("test/haha/foo.txt")?
    /// );
    /// assert_eq!(path.strip_prefix(ForwardRelativePath::new("asdf")?).is_err(), true);
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn strip_prefix<P: AsRef<ForwardRelativePath>>(
        &self,
        base: P,
    ) -> anyhow::Result<&ForwardRelativePath> {
        let base = base.as_ref();
        if base.0.is_empty() {
            Ok(self)
        } else if self.starts_with(base) {
            if self.0.len() == base.0.len() {
                Ok(ForwardRelativePath::empty())
            } else {
                Ok(ForwardRelativePath::unchecked_new(
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
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// let path = ForwardRelativePath::new("some/foo")?;
    ///
    /// assert!(path.starts_with(ForwardRelativePath::new("some/foo")?));
    /// assert!(path.starts_with(ForwardRelativePath::new("some")?));
    /// assert!(!path.starts_with(ForwardRelativePath::new("som")?));
    /// assert!(path.starts_with(ForwardRelativePath::new("")?));
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn starts_with<P: AsRef<ForwardRelativePath>>(&self, base: P) -> bool {
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
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// let path = ForwardRelativePath::new("some/foo")?;
    ///
    /// assert!(path.ends_with(ForwardRelativePath::new("some/foo")?));
    /// assert!(path.ends_with(ForwardRelativePath::new("foo")?));
    /// assert!(!path.ends_with(ForwardRelativePath::new("oo")?));
    /// assert!(path.ends_with(ForwardRelativePath::new("")?));
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn ends_with<P: AsRef<ForwardRelativePath>>(&self, child: P) -> bool {
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
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// let path = ForwardRelativePath::new("foo.rs")?;
    ///
    /// assert_eq!(Some("foo"), path.file_stem());
    /// assert_eq!(Some("foo.bar"), ForwardRelativePath::new("hi/foo.bar.rs")?.file_stem());
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
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// assert_eq!(Some("rs"), ForwardRelativePath::new("hi/foo.rs")?.extension());
    /// assert_eq!(Some("rs"), ForwardRelativePath::new("hi/foo.bar.rs")?.extension());
    /// assert_eq!(None, ForwardRelativePath::new(".git")?.extension());
    /// assert_eq!(None, ForwardRelativePath::new("foo/.git")?.extension());
    /// assert_eq!(None, ForwardRelativePath::new("")?.extension());
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
    /// use turborepo_paths::forward_relative_path::{ForwardRelativePath, ForwardRelativePathBuf};
    ///
    /// assert_eq!(
    ///     ForwardRelativePathBuf::unchecked_new("foo/baz.txt".into()),
    ///     ForwardRelativePath::new("foo/bar")?.join_normalized("../baz.txt")?,
    /// );
    ///
    /// assert_eq!(
    ///     ForwardRelativePath::new("foo")?.join_normalized("../../baz.txt").is_err(),
    ///     true
    /// );
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn join_normalized<P: AsRef<RelativePath>>(
        &self,
        path: P,
    ) -> anyhow::Result<ForwardRelativePathBuf> {
        let self_rel_path: &RelativePath = self.as_ref();
        let inner = self_rel_path.join_normalized(path.as_ref());
        ForwardRelativePathBuf::try_from(inner)
    }

    /// Append a relative system path, obtained frome e.g. `read_link`.
    ///
    /// The path will be converted to an internal path (i.e. forward slashes)
    /// before joining.
    pub fn join_system(&self, path: &Path) -> anyhow::Result<ForwardRelativePathBuf> {
        let path = fs_util::relative_path_from_system(path)?;
        self.join_normalized(path)
    }

    /// Iterator over the components of this path
    ///
    /// ```
    /// use turborepo_paths::file_name::FileName;
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// let p = ForwardRelativePath::new("foo/bar/baz")?;
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
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    ///
    /// let p = ForwardRelativePath::new("foo/bar/baz")?;
    /// assert_eq!(
    ///     p.strip_prefix_components(0),
    ///     Some(ForwardRelativePath::new("foo/bar/baz")?),
    /// );
    /// assert_eq!(
    ///     p.strip_prefix_components(1),
    ///     Some(ForwardRelativePath::new("bar/baz")?),
    /// );
    /// assert_eq!(
    ///     p.strip_prefix_components(2),
    ///     Some(ForwardRelativePath::new("baz")?),
    /// );
    /// assert_eq!(
    ///     p.strip_prefix_components(3),
    ///     Some(ForwardRelativePath::new("")?),
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
    pub fn to_buf(&self) -> ForwardRelativePathBuf {
        self.to_owned()
    }

    pub fn to_box(&self) -> Box<ForwardRelativePath> {
        self.to_buf().into_box()
    }

    /// Return a RelativePath represenation of this ForwardRelativePath.
    #[inline]
    pub fn as_relative_path(&self) -> &RelativePath {
        RelativePath::new(&self.0)
    }
}

impl ForwardRelativePathBuf {
    #[inline]
    pub fn new(s: String) -> anyhow::Result<ForwardRelativePathBuf> {
        ForwardRelativePath::new(&s)?;
        Ok(ForwardRelativePathBuf(s))
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
    /// use turborepo_paths::forward_relative_path::{ForwardRelativePath, ForwardRelativePathBuf};
    ///
    /// let mut path = ForwardRelativePathBuf::unchecked_new("foo".to_owned());
    /// path.push(ForwardRelativePath::unchecked_new("bar"));
    ///
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar".to_owned()), path);
    ///
    /// path.push(ForwardRelativePath::unchecked_new("more/file.rs"));
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar/more/file.rs".to_owned()), path);
    ///
    /// path.push(ForwardRelativePath::empty());
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar/more/file.rs".to_owned()), path);
    ///
    /// let mut path = ForwardRelativePathBuf::unchecked_new("".to_owned());
    /// path.push(ForwardRelativePath::unchecked_new("foo"));
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo".to_owned()), path);
    ///
    /// # anyhow::Ok(())
    /// ```
    pub fn push<P: AsRef<ForwardRelativePath>>(&mut self, path: P) {
        if path.as_ref().0.is_empty() {
            return;
        }

        if !self.0.is_empty() {
            self.0.push('/');
        }
        self.0.push_str(path.as_ref().as_str())
    }

    pub fn concat<'a, I: IntoIterator<Item = &'a ForwardRelativePath> + Copy>(
        items: I,
    ) -> ForwardRelativePathBuf {
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
        let mut path = ForwardRelativePathBuf::with_capacity(cap);
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
    /// use turborepo_paths::forward_relative_path::ForwardRelativePathBuf;
    /// use turborepo_paths::RelativePath;
    ///
    /// let mut path = ForwardRelativePathBuf::unchecked_new("foo".to_owned());
    /// path.push_normalized(RelativePath::new("bar"))?;
    ///
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new("more/file.rs"))?;
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar/more/file.rs".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new("../other.rs"))?;
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar/more/other.rs".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new(".."))?;
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo/bar/more".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new("../.."))?;
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("foo".to_owned()), path);
    ///
    /// path.push_normalized(RelativePath::new(".."))?;
    /// assert_eq!(ForwardRelativePathBuf::unchecked_new("".to_owned()), path);
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

    pub fn into_box(self) -> Box<ForwardRelativePath> {
        let s: Box<str> = self.0.into_boxed_str();
        ForwardRelativePath::unchecked_new_box(s)
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

impl<'a> IntoIterator for &'a ForwardRelativePath {
    type Item = &'a FileName;
    type IntoIter = ForwardRelativePathIter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> TryFrom<&'a str> for &'a ForwardRelativePath {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    /// use std::convert::TryFrom;
    ///
    /// assert!(<&ForwardRelativePath>::try_from("foo/bar").is_ok());
    /// assert!(<&ForwardRelativePath>::try_from("").is_ok());
    /// assert!(<&ForwardRelativePath>::try_from("./bar").is_err());
    /// assert!(<&ForwardRelativePath>::try_from("normalize/./bar").is_err());
    /// assert!(<&ForwardRelativePath>::try_from("/abs/bar").is_err());
    /// assert!(<&ForwardRelativePath>::try_from("normalize/../bar").is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(s: &'a str) -> anyhow::Result<&'a ForwardRelativePath> {
        ForwardRelativePathVerifier::verify_str(s)?;
        Ok(ForwardRelativePath::ref_cast(s))
    }
}

impl<'a> From<&'a FileName> for &'a ForwardRelativePath {
    #[inline]
    fn from(p: &'a FileName) -> Self {
        ForwardRelativePath::unchecked_new(p.as_str())
    }
}

impl<'a> TryFrom<&'a Path> for &'a ForwardRelativePath {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    /// use std::convert::TryFrom;
    /// use std::path::Path;
    ///
    /// assert!(<&ForwardRelativePath>::try_from(Path::new("foo/bar")).is_ok());
    /// assert!(<&ForwardRelativePath>::try_from(Path::new("")).is_ok());
    /// assert!(<&ForwardRelativePath>::try_from(Path::new("./bar")).is_err());
    /// assert!(<&ForwardRelativePath>::try_from(Path::new("normalize/./bar")).is_err());
    /// assert!(<&ForwardRelativePath>::try_from(Path::new("/abs/bar")).is_err());
    /// assert!(<&ForwardRelativePath>::try_from(Path::new("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    fn try_from(s: &'a Path) -> anyhow::Result<&'a ForwardRelativePath> {
        let s = s
            .as_os_str()
            .to_str()
            .ok_or_else(|| ForwardRelativePathError::PathNotUtf8(s.display().to_string()))?;
        ForwardRelativePathVerifier::verify_str(s)?;
        Ok(ForwardRelativePath::unchecked_new(s))
    }
}

impl<'a> TryFrom<&'a RelativePath> for &'a ForwardRelativePath {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::forward_relative_path::ForwardRelativePath;
    /// use std::convert::TryFrom;
    /// use turborepo_paths::RelativePath;
    ///
    /// assert!(<&ForwardRelativePath>::try_from(RelativePath::new("foo/bar")).is_ok());
    /// assert!(<&ForwardRelativePath>::try_from(RelativePath::new("")).is_ok());
    /// assert!(<&ForwardRelativePath>::try_from(RelativePath::new("./bar")).is_err());
    /// assert!(<&ForwardRelativePath>::try_from(RelativePath::new("normalize/./bar")).is_err());
    /// assert!(<&ForwardRelativePath>::try_from(RelativePath::new("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(p: &'a RelativePath) -> anyhow::Result<&'a ForwardRelativePath> {
        ForwardRelativePathVerifier::verify_str(p.as_str())?;
        Ok(ForwardRelativePath::unchecked_new(p.as_str()))
    }
}

impl From<ForwardRelativePathBuf> for RelativePathBuf {
    fn from(p: ForwardRelativePathBuf) -> Self {
        RelativePathBuf::from(p.0)
    }
}

impl TryFrom<String> for ForwardRelativePathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// 
    /// use turborepo_paths::forward_relative_path::ForwardRelativePathBuf;
    /// use std::convert::TryFrom;
    ///
    /// assert!(ForwardRelativePathBuf::try_from("foo/bar".to_owned()).is_ok());
    /// assert!(ForwardRelativePathBuf::try_from("".to_owned()).is_ok());
    /// assert!(ForwardRelativePathBuf::try_from("./bar".to_owned()).is_err());
    /// assert!(ForwardRelativePathBuf::try_from("normalize/./bar".to_owned()).is_err());
    /// assert!(ForwardRelativePathBuf::try_from("/abs/bar".to_owned()).is_err());
    /// assert!(ForwardRelativePathBuf::try_from("normalize/../bar".to_owned()).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(s: String) -> anyhow::Result<ForwardRelativePathBuf> {
        ForwardRelativePathVerifier::verify_str(&s)?;
        Ok(ForwardRelativePathBuf(s))
    }
}

impl TryFrom<PathBuf> for ForwardRelativePathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// use turborepo_paths::forward_relative_path::ForwardRelativePathBuf;
    /// use turborepo_paths::RelativePathBuf;
    /// use std::convert::TryFrom;
    /// use std::path::PathBuf;
    ///
    /// assert!(ForwardRelativePathBuf::try_from(PathBuf::from("foo/bar")).is_ok());
    /// assert!(ForwardRelativePathBuf::try_from(PathBuf::from("")).is_ok());
    /// assert!(ForwardRelativePathBuf::try_from(PathBuf::from("./bar")).is_err());
    /// assert!(ForwardRelativePathBuf::try_from(PathBuf::from("normalize/./bar")).is_err());
    /// assert!(ForwardRelativePathBuf::try_from(PathBuf::from("/abs/bar")).is_err());
    /// assert!(ForwardRelativePathBuf::try_from(PathBuf::from("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    fn try_from(p: PathBuf) -> anyhow::Result<ForwardRelativePathBuf> {
        // RelativePathBuf::from_path actually creates a copy.
        // avoid the copy by constructing RelativePathBuf from the underlying String
        ForwardRelativePathBuf::try_from(p.into_os_string().into_string().map_err(|_| {
            relative_path::FromPathError::from(relative_path::FromPathErrorKind::NonUtf8)
        })?)
    }
}

impl TryFrom<RelativePathBuf> for ForwardRelativePathBuf {
    type Error = anyhow::Error;

    /// no allocation conversion
    ///
    /// ```
    /// use turborepo_paths::forward_relative_path::ForwardRelativePathBuf;
    /// use turborepo_paths::RelativePathBuf;
    /// use std::convert::TryFrom;
    ///
    /// assert!(ForwardRelativePathBuf::try_from(RelativePathBuf::from("foo/bar")).is_ok());
    /// assert!(ForwardRelativePathBuf::try_from(RelativePathBuf::from("")).is_ok());
    /// assert!(ForwardRelativePathBuf::try_from(RelativePathBuf::from("./bar")).is_err());
    /// assert!(ForwardRelativePathBuf::try_from(RelativePathBuf::from("normalize/./bar")).is_err());
    /// assert!(ForwardRelativePathBuf::try_from(RelativePathBuf::from("normalize/../bar")).is_err());
    ///
    /// # anyhow::Ok(())
    /// ```
    #[inline]
    fn try_from(p: RelativePathBuf) -> anyhow::Result<ForwardRelativePathBuf> {
        ForwardRelativePathBuf::try_from(p.into_string())
    }
}

impl ToOwned for ForwardRelativePath {
    type Owned = ForwardRelativePathBuf;

    #[inline]
    fn to_owned(&self) -> ForwardRelativePathBuf {
        ForwardRelativePathBuf::unchecked_new(self.0.to_owned())
    }
}

impl AsRef<ForwardRelativePath> for ForwardRelativePath {
    #[inline]
    fn as_ref(&self) -> &ForwardRelativePath {
        self
    }
}

impl AsRef<ForwardRelativePath> for ForwardRelativePathBuf {
    #[inline]
    fn as_ref(&self) -> &ForwardRelativePath {
        ForwardRelativePath::unchecked_new(&self.0)
    }
}

impl Borrow<ForwardRelativePath> for ForwardRelativePathBuf {
    #[inline]
    fn borrow(&self) -> &ForwardRelativePath {
        self.as_ref()
    }
}

impl Deref for ForwardRelativePathBuf {
    type Target = ForwardRelativePath;

    #[inline]
    fn deref(&self) -> &ForwardRelativePath {
        ForwardRelativePath::unchecked_new(&self.0)
    }
}

/// Normalize ForwardRelativePath path if needed.
pub struct ForwardRelativePathNormalizer {}

impl ForwardRelativePathNormalizer {
    pub fn normalize_path<P: AsRef<Path> + ?Sized>(
        rel_path: &P,
    ) -> anyhow::Result<Cow<ForwardRelativePath>> {
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
            Ok(Cow::Owned(ForwardRelativePathBuf::try_from(
                normalized_path,
            )?))
        } else {
            Ok(Cow::Borrowed(ForwardRelativePath::new(path_str)?))
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

impl<'a> FromIterator<&'a FileName> for Option<ForwardRelativePathBuf> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = &'a FileName>,
    {
        from_iter::<20, _>(iter)
    }
}

impl<'a> FromIterator<&'a FileNameBuf> for Option<ForwardRelativePathBuf> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = &'a FileNameBuf>,
    {
        iter.into_iter()
            .map(<FileNameBuf as AsRef<FileName>>::as_ref)
            .collect()
    }
}

fn from_iter<'a, const N: usize, I>(iter: I) -> Option<ForwardRelativePathBuf>
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
        Some(ForwardRelativePathBuf(ret))
    }
}

#[cfg(test)]
mod tests {
    use crate::forward_relative_path::{
        from_iter, FileName, ForwardRelativePath, ForwardRelativePathBuf,
    };

    #[test]
    fn forward_path_is_comparable() -> anyhow::Result<()> {
        let path1_buf = ForwardRelativePathBuf::unchecked_new("foo".into());
        let path2_buf = ForwardRelativePathBuf::unchecked_new("foo".into());
        let path3_buf = ForwardRelativePathBuf::unchecked_new("bar".into());

        let path1 = ForwardRelativePath::new("foo")?;
        let path2 = ForwardRelativePath::new("foo")?;
        let path3 = ForwardRelativePath::new("bar")?;

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
            ForwardRelativePath::new("").unwrap(),
            AsRef::<ForwardRelativePath>::as_ref(&ForwardRelativePathBuf::concat([]))
        );
        assert_eq!(
            ForwardRelativePath::new("foo/bar/baz").unwrap(),
            AsRef::<ForwardRelativePath>::as_ref(&ForwardRelativePathBuf::concat([
                ForwardRelativePath::new("foo").unwrap(),
                ForwardRelativePath::new("bar/baz").unwrap(),
            ]))
        );
        assert_eq!(
            ForwardRelativePath::new("foo/bar/baz").unwrap(),
            AsRef::<ForwardRelativePath>::as_ref(&ForwardRelativePathBuf::concat([
                ForwardRelativePath::new("").unwrap(),
                ForwardRelativePath::new("foo").unwrap(),
                ForwardRelativePath::new("bar/baz").unwrap(),
            ]))
        );
    }

    #[test]
    fn test_from_iter() {
        let parts = &["foo", "bar", "baz"]
            .into_iter()
            .map(FileName::unchecked_new)
            .collect::<Vec<_>>();

        let expected = Some(ForwardRelativePath::unchecked_new("foo/bar/baz").to_buf());

        assert_eq!(from_iter::<1, _>(parts.iter().copied()), expected);
        assert_eq!(from_iter::<2, _>(parts.iter().copied()), expected);
        assert_eq!(from_iter::<3, _>(parts.iter().copied()), expected);
        assert_eq!(from_iter::<4, _>(parts.iter().copied()), expected);
    }
}
