use std::{
    fmt,
    path::{Components, Path},
};

use crate::{
    absolute_system_path_buf::AbsoluteSystemPathBuf, relative_system_path::RelativeSystemPath,
};

pub struct AbsoluteSystemPath<'a>(&'a Path);

impl<'a> AbsoluteSystemPath<'a> {
    /// Creates a `AbsoluteSystemPath` from a `Path` with *no* validation
    /// Note that there is no safe way to create an `AbsoluteSystemPath`
    /// because if the path separators need to be replaced, that would
    /// require allocating a new `PathBuf`, which we cannot do.
    ///
    /// # Arguments
    ///
    /// * `path`:
    ///
    /// returns: AbsoluteSystemPath
    ///
    /// # Examples
    ///
    /// ```
    ///  use std::path::Path;
    ///  let path = AbsoluteSystemPath::new_unchecked(Path::new("/foo/bar"));
    ///  assert_eq!(path.to_str(), Some("/foo/bar"));
    ///  assert_eq!(path.file_name(), Some("bar"));
    ///  // Unsafe!
    ///  let path = AbsoluteSystemPath::new_unchecked(Path::new("./foo/"));
    ///  assert_eq!(path.to_str(), Some("./foo/"));
    /// ```
    pub fn new_unchecked(path: &'a Path) -> Self {
        AbsoluteSystemPath(path)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn components(&self) -> Components<'a> {
        self.0.components()
    }

    pub fn parent(&self) -> Option<Self> {
        self.0.parent().map(AbsoluteSystemPath::new_unchecked)
    }

    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.0.starts_with(base.as_ref())
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.0.ends_with(child.as_ref())
    }

    pub fn join(&self, path: &RelativeSystemPath) -> AbsoluteSystemPathBuf {
        let mut new_path = self.0.to_path_buf();
        new_path.push(path.as_path());
        AbsoluteSystemPathBuf::new_unchecked(new_path)
    }

    pub fn to_str(&self) -> Option<&str> {
        self.0.to_str()
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.file_name().and_then(|s| s.to_str())
    }

    pub fn extension(&self) -> Option<&str> {
        self.0.extension().and_then(|s| s.to_str())
    }
}

impl<'a> fmt::Display for AbsoluteSystemPath<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

impl<'a> fmt::Debug for AbsoluteSystemPath<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a> PartialEq for AbsoluteSystemPath<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
