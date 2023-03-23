use std::{
    borrow::{Borrow, Cow},
    cmp::Ordering,
    collections::TryReserveError,
    error,
    ffi::{OsStr, OsString},
    fmt,
    fs::{Metadata, ReadDir},
    hash::Hash,
    io::{self, Result},
    iter::{self, FusedIterator},
    ops::{Deref, DerefMut},
    path::{Components, Display, Iter, Path, PathBuf, StripPrefixError},
    rc::Rc,
    result::Result as StdResult,
    str::FromStr,
    sync::Arc,
};

use delegate::delegate;

#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct AnchoredSystemPathBuf(PathBuf);

impl AnchoredSystemPathBuf {
    #[must_use]
    pub fn as_anchored_system_path(&self) -> &AnchoredSystemPath {
        unsafe { AnchoredSystemPath::coerce_anchored_system_path(&self.0) }
    }

    #[must_use]
    pub fn into_boxed_anchored_system_path(self) -> Box<AnchoredSystemPath> {
        let ptr = Box::into_raw(self.0.into_boxed_path()) as *mut AnchoredSystemPath;
        unsafe { Box::from_raw(ptr) }
    }

    #[must_use]
    pub fn from_path_buf(path: PathBuf) -> StdResult<AnchoredSystemPathBuf, FromError> {
        if !path.is_absolute() {
            Ok(AnchoredSystemPathBuf(path))
        } else {
            Err(FromError(()))
        }
    }

    // API OVERRIDES

    pub fn push<P: AsRef<AnchoredSystemPath>>(&mut self, path: P) {
        self.0.push(path.as_ref())
    }

    // This is a static method, it can't be delegated.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf(PathBuf::with_capacity(capacity))
    }

    delegate! {
        to self.0 {
            pub fn as_path(&self) -> &Path;
            pub fn capacity(&self) -> usize;
            pub fn clear(&mut self);
            pub fn into_boxed_path(self) -> Box<Path>;
            pub fn into_os_string(self) -> OsString;
            pub fn pop(&mut self) -> bool;
            // OVERRIDE pub fn push<P: AsRef<Path>>(&mut self, path: P);
            pub fn reserve(&mut self, additional: usize);
            pub fn reserve_exact(&mut self, additional: usize);
            pub fn set_extension<S: AsRef<OsStr>>(&mut self, extension: S) -> bool;
            pub fn set_file_name<S: AsRef<OsStr>>(&mut self, file_name: S);
            pub fn shrink_to(&mut self, min_capacity: usize);
            pub fn shrink_to_fit(&mut self);
            pub fn try_reserve(&mut self, additional: usize) -> StdResult<(), TryReserveError>;
            pub fn try_reserve_exact(&mut self, additional: usize) -> StdResult<(), TryReserveError>;
        }
    }
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct AnchoredSystemPath(Path);

impl AnchoredSystemPath {
    pub fn new(s: &(impl AsRef<OsStr> + ?Sized)) -> StdResult<&AnchoredSystemPath, FromError> {
        let path = Path::new(s.as_ref());
        if !path.is_absolute() {
            Ok(unsafe { AnchoredSystemPath::coerce_anchored_system_path(path) })
        } else {
            Err(FromError(()))
        }
    }

    // MANUAL IMPLEMENTATIONS

    #[must_use]
    pub fn into_anchored_system_path_buf(self: Box<AnchoredSystemPath>) -> AnchoredSystemPathBuf {
        let ptr = Box::into_raw(self) as *mut Path;
        let boxed_path = unsafe { Box::from_raw(ptr) };
        AnchoredSystemPathBuf(boxed_path.into_path_buf())
    }

    #[must_use]
    pub fn to_anchored_system_path_buf(&self) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf(self.0.to_path_buf())
    }

    #[must_use]
    pub fn from_path(path: &Path) -> StdResult<&AnchoredSystemPath, FromError> {
        AnchoredSystemPath::new(path.as_os_str())
    }

    #[must_use]
    unsafe fn coerce_anchored_system_path(path: &Path) -> &AnchoredSystemPath {
        &*(path as *const Path as *const AnchoredSystemPath)
    }

    unsafe fn coerce_anchored_system_path_mut(path: &mut Path) -> &mut AnchoredSystemPath {
        &mut *(path as *mut Path as *mut AnchoredSystemPath)
    }

    // API OVERRIDES
    // These explicitly change the method signature.

    #[inline]
    pub fn ancestors(&self) -> AnchoredSystemPathAncestors<'_> {
        AnchoredSystemPathAncestors { next: Some(&self) }
    }

    #[must_use]
    pub fn into_path_buf(self: Box<AnchoredSystemPath>) -> PathBuf {
        let ptr = Box::into_raw(self) as *mut Path;
        let boxed_path = unsafe { Box::from_raw(ptr) };
        boxed_path.into_path_buf()
    }

    #[must_use]
    pub fn join<P: AsRef<AnchoredSystemPath>>(&self, path: P) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf(self.0.join(&path.as_ref()))
    }

    #[must_use]
    pub fn parent(&self) -> Option<&AnchoredSystemPath> {
        self.0
            .parent()
            .and_then(|path| Some(unsafe { AnchoredSystemPath::coerce_anchored_system_path(path) }))
    }

    pub fn with_extension<S: AsRef<OsStr>>(&self, extension: S) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf(self.0.with_extension(extension.as_ref()))
    }

    #[must_use]
    pub fn with_file_name<S: AsRef<OsStr>>(&self, file_name: S) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf(self.0.with_file_name(file_name.as_ref()))
    }

    delegate! {
        to self.0 {
            // OVERRIDE pub fn ancestors(&self) -> Ancestors<'_>;
            pub fn as_os_str(&self) -> &OsStr;
            // INVALID pub fn canonicalize(&self) -> Result<PathBuf>;
            pub fn components(&self) -> Components<'_>;
            pub fn display(&self) -> Display<'_>;
            pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool;
            pub fn exists(&self) -> bool;
            pub fn extension(&self) -> Option<&OsStr>;
            pub fn file_name(&self) -> Option<&OsStr>;
            pub fn file_stem(&self) -> Option<&OsStr>;
            pub fn has_root(&self) -> bool;
            // OVERRIDE pub fn into_path_buf(self: Box<Path>) -> PathBuf;
            pub fn is_absolute(&self) -> bool;
            pub fn is_dir(&self) -> bool;
            pub fn is_file(&self) -> bool;
            pub fn is_relative(&self) -> bool;
            pub fn is_symlink(&self) -> bool;
            pub fn iter(&self) -> Iter<'_>;
            // OVERRIDE pub fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf;
            pub fn metadata(&self) -> Result<Metadata>;
            // OVERRIDE pub fn parent(&self) -> Option<&Path>;
            pub fn read_dir(&self) -> Result<ReadDir>;
            pub fn read_link(&self) -> Result<PathBuf>;
            pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool;
            pub fn strip_prefix<P>(&self, base: P) -> StdResult<&Path, StripPrefixError>
                where P: AsRef<Path>;
            pub fn symlink_metadata(&self) -> Result<Metadata>;
            pub fn to_path_buf(&self) -> PathBuf;
            pub fn to_str(&self) -> Option<&str>;
            pub fn to_string_lossy(&self) -> Cow<'_, str>;
            pub fn try_exists(&self) -> Result<bool>;
            // OVERRIDE pub fn with_extension<S: AsRef<OsStr>>(&self, extension: S) -> PathBuf;
            // OVERRIDE pub fn with_file_name<S: AsRef<OsStr>>(&self, file_name: S) -> PathBuf;
        }
    }
}

// Ancestors
// All ancestors of an AnchoredSystemPath are _also_ AnchoredSystemPaths.

#[derive(Copy, Clone, Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct AnchoredSystemPathAncestors<'a> {
    next: Option<&'a AnchoredSystemPath>,
}

impl<'a> Iterator for AnchoredSystemPathAncestors<'a> {
    type Item = &'a AnchoredSystemPath;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.next;
        self.next = next.and_then(AnchoredSystemPath::parent);
        next
    }
}

impl FusedIterator for AnchoredSystemPathAncestors<'_> {}

// Direct Iteration

impl<'a> IntoIterator for &'a AnchoredSystemPathBuf {
    type Item = &'a OsStr;
    type IntoIter = Iter<'a>;
    #[inline]
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a AnchoredSystemPath {
    type Item = &'a OsStr;
    type IntoIter = Iter<'a>;
    #[inline]
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<P: AsRef<AnchoredSystemPath>> iter::Extend<P> for AnchoredSystemPathBuf {
    fn extend<I: IntoIterator<Item = P>>(&mut self, iter: I) {
        iter.into_iter().for_each(move |p| self.push(p.as_ref()));
    }
}

// AsRef
// Only the things which absolutely cannot fail.

impl AsRef<AnchoredSystemPath> for AnchoredSystemPath {
    #[inline]
    fn as_ref(&self) -> &AnchoredSystemPath {
        self
    }
}

impl AsRef<AnchoredSystemPath> for AnchoredSystemPathBuf {
    #[inline]
    fn as_ref(&self) -> &AnchoredSystemPath {
        self.as_anchored_system_path()
    }
}

impl AsRef<OsStr> for AnchoredSystemPath {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl AsRef<OsStr> for AnchoredSystemPathBuf {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self.0.as_os_str()
    }
}

impl AsRef<Path> for AnchoredSystemPath {
    #[inline]
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for AnchoredSystemPathBuf {
    #[inline]
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

// Borrowing

impl Deref for AnchoredSystemPathBuf {
    type Target = AnchoredSystemPath;

    #[inline]
    fn deref(&self) -> &AnchoredSystemPath {
        self.as_anchored_system_path()
    }
}

impl DerefMut for AnchoredSystemPathBuf {
    #[inline]
    fn deref_mut(&mut self) -> &mut AnchoredSystemPath {
        unsafe { AnchoredSystemPath::coerce_anchored_system_path_mut(&mut self.0) }
    }
}

impl Borrow<AnchoredSystemPath> for AnchoredSystemPathBuf {
    #[inline]
    fn borrow(&self) -> &AnchoredSystemPath {
        self.deref()
    }
}

impl ToOwned for AnchoredSystemPath {
    type Owned = AnchoredSystemPathBuf;

    #[inline]
    fn to_owned(&self) -> AnchoredSystemPathBuf {
        self.to_anchored_system_path_buf()
    }
}

// Clone

impl Clone for Box<AnchoredSystemPath> {
    #[inline]
    fn clone(&self) -> Self {
        self.to_anchored_system_path_buf()
            .into_boxed_anchored_system_path()
    }
}

// From<AnchoredSystemPath(Buf)> for T

impl<T: ?Sized + AsRef<AnchoredSystemPath>> From<&T> for AnchoredSystemPathBuf {
    fn from(s: &T) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf::from(s.as_ref().to_owned())
    }
}

impl From<AnchoredSystemPathBuf> for PathBuf {
    fn from(path: AnchoredSystemPathBuf) -> PathBuf {
        path.0
    }
}

impl From<&AnchoredSystemPath> for Arc<Path> {
    /// Converts a [`AnchoredSystemPath`] into an [`Arc`] by copying the
    /// [`AnchoredSystemPath`] data into a new [`Arc`] buffer.
    #[inline]
    fn from(s: &AnchoredSystemPath) -> Arc<Path> {
        let arc: Arc<OsStr> = Arc::from(s.as_os_str());
        unsafe { Arc::from_raw(Arc::into_raw(arc) as *const Path) }
    }
}

impl From<&AnchoredSystemPath> for Arc<AnchoredSystemPath> {
    /// Converts a [`AnchoredSystemPath`] into an [`Arc`] by copying the
    /// [`AnchoredSystemPath`] data into a new [`Arc`] buffer.
    #[inline]
    fn from(s: &AnchoredSystemPath) -> Arc<AnchoredSystemPath> {
        let arc: Arc<OsStr> = Arc::from(s.as_os_str());
        unsafe { Arc::from_raw(Arc::into_raw(arc) as *const AnchoredSystemPath) }
    }
}

impl From<&AnchoredSystemPath> for Box<Path> {
    /// Creates a boxed [`Path`] from a reference.
    ///
    /// This will allocate and clone `path` to it.
    fn from(path: &AnchoredSystemPath) -> Box<Path> {
        let boxed: Box<OsStr> = path.as_os_str().into();
        let rw = Box::into_raw(boxed) as *mut Path;
        unsafe { Box::from_raw(rw) }
    }
}

impl From<&AnchoredSystemPath> for Box<AnchoredSystemPath> {
    /// Creates a boxed [`AnchoredSystemPath`] from a reference.
    ///
    /// This will allocate and clone `path` to it.
    fn from(path: &AnchoredSystemPath) -> Box<AnchoredSystemPath> {
        let boxed: Box<OsStr> = path.as_os_str().into();
        let rw = Box::into_raw(boxed) as *mut AnchoredSystemPath;
        unsafe { Box::from_raw(rw) }
    }
}

impl From<&AnchoredSystemPath> for Rc<Path> {
    /// Converts a [`AnchoredSystemPath`] into an [`Rc`] by copying the
    /// [`AnchoredSystemPath`] data into a new [`Rc`] buffer.
    #[inline]
    fn from(s: &AnchoredSystemPath) -> Rc<Path> {
        let rc: Rc<OsStr> = Rc::from(s.as_os_str());
        unsafe { Rc::from_raw(Rc::into_raw(rc) as *const Path) }
    }
}

impl From<&AnchoredSystemPath> for Rc<AnchoredSystemPath> {
    /// Converts a [`AnchoredSystemPath`] into an [`Rc`] by copying the
    /// [`AnchoredSystemPath`] data into a new [`Rc`] buffer.
    #[inline]
    fn from(s: &AnchoredSystemPath) -> Rc<AnchoredSystemPath> {
        let rc: Rc<OsStr> = Rc::from(s.as_os_str());
        unsafe { Rc::from_raw(Rc::into_raw(rc) as *const AnchoredSystemPath) }
    }
}

impl From<Box<AnchoredSystemPath>> for PathBuf {
    /// Converts a <code>[Box]&lt;[AnchoredSystemPath]&gt;</code> into a
    /// [`PathBuf`].
    ///
    /// This conversion does not allocate or copy memory.
    #[inline]
    fn from(boxed: Box<AnchoredSystemPath>) -> PathBuf {
        boxed.into_path_buf()
    }
}

impl From<Box<AnchoredSystemPath>> for AnchoredSystemPathBuf {
    /// Converts a <code>[Box]&lt;[AnchoredSystemPath]&gt;</code> into a
    /// [`AnchoredSystemPathBuf`].
    ///
    /// This conversion does not allocate or copy memory.
    #[inline]
    fn from(boxed: Box<AnchoredSystemPath>) -> AnchoredSystemPathBuf {
        boxed.into_anchored_system_path_buf()
    }
}

impl From<AnchoredSystemPathBuf> for Arc<Path> {
    /// Converts a [`AnchoredSystemPathBuf`] into an <code>[Arc]<[Path]></code>
    /// by moving the [`AnchoredSystemPathBuf`] data into a new [`Arc`]
    /// buffer.
    #[inline]
    fn from(s: AnchoredSystemPathBuf) -> Arc<Path> {
        let arc: Arc<OsStr> = Arc::from(s.into_os_string());
        unsafe { Arc::from_raw(Arc::into_raw(arc) as *const Path) }
    }
}

impl From<AnchoredSystemPathBuf> for Arc<AnchoredSystemPath> {
    /// Converts a [`AnchoredSystemPathBuf`] into an
    /// <code>[Arc]<[AnchoredSystemPath]></code> by moving the
    /// [`AnchoredSystemPathBuf`] data into a new [`Arc`] buffer.
    #[inline]
    fn from(s: AnchoredSystemPathBuf) -> Arc<AnchoredSystemPath> {
        let arc: Arc<OsStr> = Arc::from(s.into_os_string());
        unsafe { Arc::from_raw(Arc::into_raw(arc) as *const AnchoredSystemPath) }
    }
}

impl From<AnchoredSystemPathBuf> for Box<Path> {
    /// Converts a [`AnchoredSystemPathBuf`] into a
    /// <code>[Box]&lt;[Path]&gt;</code>.
    ///
    /// This conversion currently should not allocate memory,
    /// but this behavior is not guaranteed on all platforms or in all future
    /// versions.
    #[inline]
    fn from(p: AnchoredSystemPathBuf) -> Box<Path> {
        p.into_boxed_path()
    }
}

impl From<AnchoredSystemPathBuf> for Box<AnchoredSystemPath> {
    /// Converts a [`AnchoredSystemPathBuf`] into a
    /// <code>[Box]&lt;[AnchoredSystemPath]&gt;</code>.
    ///
    /// This conversion currently should not allocate memory,
    /// but this behavior is not guaranteed on all platforms or in all future
    /// versions.
    #[inline]
    fn from(p: AnchoredSystemPathBuf) -> Box<AnchoredSystemPath> {
        p.into_boxed_anchored_system_path()
    }
}

impl From<AnchoredSystemPathBuf> for OsString {
    /// Converts a [`AnchoredSystemPathBuf`] into an [`OsString`]
    ///
    /// This conversion does not allocate or copy memory.
    #[inline]
    fn from(path_buf: AnchoredSystemPathBuf) -> OsString {
        path_buf.0.into_os_string()
    }
}

impl From<AnchoredSystemPathBuf> for Rc<Path> {
    /// Converts a [`AnchoredSystemPathBuf`] into an <code>[Rc]<[Path]></code>
    /// by moving the [`AnchoredSystemPathBuf`] data into a new [`Rc`]
    /// buffer.
    #[inline]
    fn from(s: AnchoredSystemPathBuf) -> Rc<Path> {
        let rc: Rc<OsStr> = Rc::from(s.into_os_string());
        unsafe { Rc::from_raw(Rc::into_raw(rc) as *const Path) }
    }
}

impl From<AnchoredSystemPathBuf> for Rc<AnchoredSystemPath> {
    /// Converts a [`AnchoredSystemPathBuf`] into an
    /// <code>[Rc]<[AnchoredSystemPath]></code> by moving the
    /// [`AnchoredSystemPathBuf`] data into a new [`Rc`] buffer.
    #[inline]
    fn from(s: AnchoredSystemPathBuf) -> Rc<AnchoredSystemPath> {
        let rc: Rc<OsStr> = Rc::from(s.into_os_string());
        unsafe { Rc::from_raw(Rc::into_raw(rc) as *const AnchoredSystemPath) }
    }
}

impl<'a> From<AnchoredSystemPathBuf> for Cow<'a, Path> {
    /// Creates a clone-on-write pointer from an owned
    /// instance of [`PathBuf`].
    ///
    /// This conversion does not clone or allocate.
    #[inline]
    fn from(s: AnchoredSystemPathBuf) -> Cow<'a, Path> {
        Cow::Owned(s.0)
    }
}

impl<'a> From<AnchoredSystemPathBuf> for Cow<'a, AnchoredSystemPath> {
    /// Creates a clone-on-write pointer from an owned
    /// instance of [`PathBuf`].
    ///
    /// This conversion does not clone or allocate.
    #[inline]
    fn from(s: AnchoredSystemPathBuf) -> Cow<'a, AnchoredSystemPath> {
        Cow::Owned(s)
    }
}

impl<'a> From<Cow<'a, AnchoredSystemPath>> for AnchoredSystemPathBuf {
    /// Converts a clone-on-write pointer to an owned path.
    ///
    /// Converting from a `Cow::Owned` does not clone or allocate.
    #[inline]
    fn from(p: Cow<'a, AnchoredSystemPath>) -> Self {
        p.into_owned()
    }
}

impl<'a> From<&'a AnchoredSystemPathBuf> for Cow<'a, Path> {
    /// Creates a clone-on-write pointer from a reference to
    /// [`AnchoredSystemPathBuf`].
    ///
    /// This conversion does not clone or allocate.
    #[inline]
    fn from(p: &'a AnchoredSystemPathBuf) -> Cow<'a, Path> {
        Cow::Borrowed(p.as_path())
    }
}

impl<'a> From<&'a AnchoredSystemPathBuf> for Cow<'a, AnchoredSystemPath> {
    /// Creates a clone-on-write pointer from a reference to
    /// [`AnchoredSystemPathBuf`].
    ///
    /// This conversion does not clone or allocate.
    #[inline]
    fn from(p: &'a AnchoredSystemPathBuf) -> Cow<'a, AnchoredSystemPath> {
        Cow::Borrowed(p.as_anchored_system_path())
    }
}

impl<'a> From<&'a AnchoredSystemPath> for Cow<'a, AnchoredSystemPath> {
    /// Creates a clone-on-write pointer from a reference to
    /// [`AnchoredSystemPath`].
    ///
    /// This conversion does not clone or allocate.
    #[inline]
    fn from(s: &'a AnchoredSystemPath) -> Cow<'a, AnchoredSystemPath> {
        Cow::Borrowed(s)
    }
}

impl<'a> From<&'a AnchoredSystemPath> for Cow<'a, Path> {
    /// Creates a clone-on-write pointer from a reference to
    /// [`AnchoredSystemPath`].
    ///
    /// This conversion does not clone or allocate.
    #[inline]
    fn from(s: &'a AnchoredSystemPath) -> Cow<'a, Path> {
        Cow::Borrowed(s.as_ref())
    }
}

impl From<Cow<'_, AnchoredSystemPath>> for Box<AnchoredSystemPath> {
    /// Creates a boxed [`AnchoredSystemPath`] from a clone-on-write pointer.
    ///
    /// Converting from a `Cow::Owned` does not clone or allocate.
    #[inline]
    fn from(cow: Cow<'_, AnchoredSystemPath>) -> Box<AnchoredSystemPath> {
        match cow {
            Cow::Borrowed(path) => Box::from(path),
            Cow::Owned(path) => Box::from(path),
        }
    }
}

// TryFrom<T> for AnchoredSystemPath(Buf)

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct FromError(());

impl FromError {
    /// Converts self into a [`std::io::Error`] with kind
    /// [`InvalidData`](io::ErrorKind::InvalidData).
    ///
    /// Many users of `FromError` will want to convert it into an `io::Error`.
    /// This is a convenience method to do that.
    pub fn into_io_error(self) -> io::Error {
        // NOTE: we don't currently implement `From<FromError> for io::Error` because we
        // want to ensure the user actually desires that conversion.
        io::Error::new(io::ErrorKind::InvalidData, self)
    }
}

impl fmt::Display for FromError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Path is not an AnchoredSystemPath")
    }
}

impl error::Error for FromError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        None
    }
}

impl TryFrom<PathBuf> for AnchoredSystemPathBuf {
    type Error = FromError;

    fn try_from(path: PathBuf) -> StdResult<AnchoredSystemPathBuf, Self::Error> {
        AnchoredSystemPathBuf::from_path_buf(path)
    }
}

impl<'a> TryFrom<&'a Path> for &'a AnchoredSystemPath {
    type Error = FromError;

    fn try_from(path: &'a Path) -> StdResult<&'a AnchoredSystemPath, Self::Error> {
        AnchoredSystemPath::from_path(path)
    }
}

impl TryFrom<OsString> for AnchoredSystemPathBuf {
    type Error = FromError;

    fn try_from(path: OsString) -> StdResult<AnchoredSystemPathBuf, Self::Error> {
        AnchoredSystemPathBuf::from_path_buf(path.into())
    }
}

impl TryFrom<String> for AnchoredSystemPathBuf {
    type Error = FromError;

    fn try_from(path: String) -> StdResult<AnchoredSystemPathBuf, Self::Error> {
        AnchoredSystemPathBuf::from_path_buf(path.into())
    }
}

impl TryFrom<&str> for AnchoredSystemPathBuf {
    type Error = FromError;

    fn try_from(path: &str) -> StdResult<AnchoredSystemPathBuf, Self::Error> {
        AnchoredSystemPathBuf::from_path_buf(path.into())
    }
}

impl FromStr for AnchoredSystemPathBuf {
    type Err = FromError;

    #[inline]
    fn from_str(path: &str) -> StdResult<AnchoredSystemPathBuf, Self::Err> {
        AnchoredSystemPathBuf::from_path_buf(path.into())
    }
}

// Comparison
macro_rules! impl_cmp {
    (<$($life:lifetime),*> $lhs:ty, $rhs: ty) => {
        impl<$($life),*> PartialEq<$rhs> for $lhs {
            #[inline]
            fn eq(&self, other: &$rhs) -> bool {
                <AnchoredSystemPath as PartialEq>::eq(self, other)
            }
        }

        impl<$($life),*> PartialEq<$lhs> for $rhs {
            #[inline]
            fn eq(&self, other: &$lhs) -> bool {
                <AnchoredSystemPath as PartialEq>::eq(self, other)
            }
        }

        impl<$($life),*> PartialOrd<$rhs> for $lhs {
            #[inline]
            fn partial_cmp(&self, other: &$rhs) -> Option<Ordering> {
                <AnchoredSystemPath as PartialOrd>::partial_cmp(self, other)
            }
        }

        impl<$($life),*> PartialOrd<$lhs> for $rhs {
            #[inline]
            fn partial_cmp(&self, other: &$lhs) -> Option<Ordering> {
                <AnchoredSystemPath as PartialOrd>::partial_cmp(self, other)
            }
        }
    };
}

impl_cmp!(<> AnchoredSystemPathBuf, AnchoredSystemPath);
impl_cmp!(<'a> AnchoredSystemPathBuf, &'a AnchoredSystemPath);
impl_cmp!(<'a> Cow<'a, AnchoredSystemPath>, AnchoredSystemPath);
impl_cmp!(<'a, 'b> Cow<'a, AnchoredSystemPath>, &'b AnchoredSystemPath);
impl_cmp!(<'a> Cow<'a, AnchoredSystemPath>, AnchoredSystemPathBuf);

macro_rules! impl_cmp_std_path {
    (<$($life:lifetime),*> $lhs:ty, $rhs: ty) => {
        impl<$($life),*> PartialEq<$rhs> for $lhs {
            #[inline]
            fn eq(&self, other: &$rhs) -> bool {
                <Path as PartialEq>::eq(self.as_ref(), other)
            }
        }

        impl<$($life),*> PartialEq<$lhs> for $rhs {
            #[inline]
            fn eq(&self, other: &$lhs) -> bool {
                <Path as PartialEq>::eq(self, other.as_ref())
            }
        }

        impl<$($life),*> PartialOrd<$rhs> for $lhs {
            #[inline]
            fn partial_cmp(&self, other: &$rhs) -> Option<Ordering> {
                <Path as PartialOrd>::partial_cmp(self.as_ref(), other)
            }
        }

        impl<$($life),*> PartialOrd<$lhs> for $rhs {
            #[inline]
            fn partial_cmp(&self, other: &$lhs) -> Option<Ordering> {
                <Path as PartialOrd>::partial_cmp(self, other.as_ref())
            }
        }
    };
}

impl_cmp_std_path!(<> AnchoredSystemPathBuf, Path);
impl_cmp_std_path!(<'a> AnchoredSystemPathBuf, &'a Path);
impl_cmp_std_path!(<'a> AnchoredSystemPathBuf, Cow<'a, Path>);
impl_cmp_std_path!(<> AnchoredSystemPathBuf, PathBuf);
impl_cmp_std_path!(<> AnchoredSystemPath, Path);
impl_cmp_std_path!(<'a> AnchoredSystemPath, &'a Path);
impl_cmp_std_path!(<'a> AnchoredSystemPath, Cow<'a, Path>);
impl_cmp_std_path!(<> AnchoredSystemPath, PathBuf);
impl_cmp_std_path!(<'a> &'a AnchoredSystemPath, Path);
impl_cmp_std_path!(<'a, 'b> &'a AnchoredSystemPath, Cow<'b, Path>);
impl_cmp_std_path!(<'a> &'a AnchoredSystemPath, PathBuf);

macro_rules! impl_cmp_os_str {
    (<$($life:lifetime),*> $lhs:ty, $rhs: ty) => {
        impl<$($life),*> PartialEq<$rhs> for $lhs {
            #[inline]
            fn eq(&self, other: &$rhs) -> bool {
                <Path as PartialEq>::eq(self.as_ref(), other.as_ref())
            }
        }

        impl<$($life),*> PartialEq<$lhs> for $rhs {
            #[inline]
            fn eq(&self, other: &$lhs) -> bool {
                <Path as PartialEq>::eq(self.as_ref(), other.as_ref())
            }
        }

        impl<$($life),*> PartialOrd<$rhs> for $lhs {
            #[inline]
            fn partial_cmp(&self, other: &$rhs) -> Option<Ordering> {
                <Path as PartialOrd>::partial_cmp(self.as_ref(), other.as_ref())
            }
        }

        impl<$($life),*> PartialOrd<$lhs> for $rhs {
            #[inline]
            fn partial_cmp(&self, other: &$lhs) -> Option<Ordering> {
                <Path as PartialOrd>::partial_cmp(self.as_ref(), other.as_ref())
            }
        }
    };
}

impl_cmp_os_str!(<> AnchoredSystemPathBuf, OsStr);
impl_cmp_os_str!(<'a> AnchoredSystemPathBuf, &'a OsStr);
impl_cmp_os_str!(<'a> AnchoredSystemPathBuf, Cow<'a, OsStr>);
impl_cmp_os_str!(<> AnchoredSystemPathBuf, OsString);
impl_cmp_os_str!(<> AnchoredSystemPath, OsStr);
impl_cmp_os_str!(<'a> AnchoredSystemPath, &'a OsStr);
impl_cmp_os_str!(<'a> AnchoredSystemPath, Cow<'a, OsStr>);
impl_cmp_os_str!(<> AnchoredSystemPath, OsString);
impl_cmp_os_str!(<'a> &'a AnchoredSystemPath, OsStr);
impl_cmp_os_str!(<'a, 'b> &'a AnchoredSystemPath, Cow<'b, OsStr>);
impl_cmp_os_str!(<'a> &'a AnchoredSystemPath, OsString);
