use std::path::{Components, Path, PathBuf};

use crate::{IntoUnix, PathValidationError};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RelativeUnixPathBuf(PathBuf);

impl RelativeUnixPathBuf {
    /// Create a new RelativeUnixPathBuf from a PathBuf by calling `into_unix()`
    ///
    /// NOTE: `into_unix` *only* converts Windows paths to Unix paths *on* a
    /// Windows system. Do not pass a Windows path on a Unix system and
    /// assume it'll be converted.
    ///
    /// # Arguments
    ///
    /// * `path`:
    ///
    /// returns: Result<RelativeUnixPathBuf, PathValidationError>
    ///
    /// # Examples
    ///
    /// ```
    /// ```
    pub fn new(path: PathBuf) -> Result<Self, PathValidationError> {
        if path.is_absolute() {
            return Err(PathValidationError::NotRelative(path));
        }

        Ok(RelativeUnixPathBuf(path.into_unix()?))
    }

    pub fn new_unchecked(path: PathBuf) -> Self {
        RelativeUnixPathBuf(path)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn components(&self) -> Components<'_> {
        self.0.components()
    }

    pub fn parent(&self) -> Option<Self> {
        self.0
            .parent()
            .map(|p| RelativeUnixPathBuf(p.to_path_buf()))
    }

    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.0.starts_with(base.as_ref())
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.0.ends_with(child.as_ref())
    }

    pub fn join<P: AsRef<Path>>(&self, path: P) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf(self.0.join(path))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_unix_path_buf() {
        let path = RelativeUnixPathBuf::new(PathBuf::from("foo/bar")).unwrap();
        assert_eq!(path.as_path(), Path::new("foo/bar"));
        assert_eq!(path.components().count(), 2);
        assert_eq!(path.parent().unwrap().as_path(), Path::new("foo"));
        assert!(path.starts_with("foo"));
        assert!(path.ends_with("bar"));
        assert_eq!(path.join("baz").as_path(), Path::new("foo/bar/baz"));
        assert_eq!(path.to_str(), Some("foo/bar"));
        assert_eq!(path.file_name(), Some("bar"));
        assert_eq!(path.extension(), None);
    }

    #[test]
    fn test_relative_unix_path_buf_with_extension() {
        let path = RelativeUnixPathBuf::new(PathBuf::from("foo/bar.txt")).unwrap();
        assert_eq!(path.as_path(), Path::new("foo/bar.txt"));
        assert_eq!(path.components().count(), 2);
        assert_eq!(path.parent().unwrap().as_path(), Path::new("foo"));
        assert!(path.starts_with("foo"));
        assert!(path.ends_with("bar.txt"));
        assert_eq!(path.join("baz").as_path(), Path::new("foo/bar.txt/baz"));
        assert_eq!(path.to_str(), Some("foo/bar.txt"));
        assert_eq!(path.file_name(), Some("bar.txt"));
        assert_eq!(path.extension(), Some("txt"));
    }

    #[test]
    fn test_relative_unix_path_buf_errors() {
        assert!(RelativeUnixPathBuf::new(PathBuf::from("/foo/bar")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_convert_from_windows_path() {
        let path = RelativeUnixPathBuf::new(PathBuf::from("foo\\bar")).unwrap();
        assert_eq!(path.as_path(), Path::new("foo/bar"));
    }
}
