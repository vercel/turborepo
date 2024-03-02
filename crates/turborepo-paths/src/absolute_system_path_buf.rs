use std::{
    borrow::Borrow,
    fmt, io,
    ops::Deref,
    path::{Path, PathBuf},
};

use camino::{Utf8Components, Utf8Path, Utf8PathBuf};
use fs_err as fs;
use path_clean::PathClean;
use serde::Serialize;

use crate::{AbsoluteSystemPath, AnchoredSystemPathBuf, PathError};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
pub struct AbsoluteSystemPathBuf(pub(crate) Utf8PathBuf);

impl Borrow<AbsoluteSystemPath> for AbsoluteSystemPathBuf {
    fn borrow(&self) -> &AbsoluteSystemPath {
        let path = self.as_path();
        unsafe { &*(path as *const Utf8Path as *const AbsoluteSystemPath) }
    }
}

impl AsRef<AbsoluteSystemPath> for AbsoluteSystemPathBuf {
    fn as_ref(&self) -> &AbsoluteSystemPath {
        self
    }
}

impl Deref for AbsoluteSystemPathBuf {
    type Target = AbsoluteSystemPath;

    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl AbsoluteSystemPathBuf {
    /// Create a new AbsoluteSystemPathBuf from `unchecked_path`.
    /// Confirms that `unchecked_path` is absolute. Does *not* convert
    /// to system path, since that is generally undecidable (see module
    /// documentation)
    ///
    /// # Arguments
    ///
    /// * `unchecked_path`: The path to be validated and converted to an
    ///   `AbsoluteSystemPathBuf`.
    ///
    /// returns: Result<AbsoluteSystemPathBuf, PathError>
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::{Path, PathBuf};
    /// use camino::Utf8Path;
    /// use turbopath::AbsoluteSystemPathBuf;
    /// #[cfg(windows)]
    /// let path = "C:/Users/user";
    /// #[cfg(not(windows))]
    /// let path = "/Users/user";
    ///
    /// let absolute_path = AbsoluteSystemPathBuf::new(path).unwrap();
    ///
    /// #[cfg(windows)]
    /// assert_eq!(absolute_path.as_path(), Utf8Path::new("C:\\Users\\user"));
    /// #[cfg(not(windows))]
    /// assert_eq!(absolute_path.as_path(), Utf8Path::new("/Users/user"));
    /// ```
    pub fn new(unchecked_path: impl Into<String>) -> Result<Self, PathError> {
        let unchecked_path = unchecked_path.into();
        if !Path::new(&unchecked_path).is_absolute() {
            return Err(PathError::NotAbsolute(unchecked_path));
        }
        Ok(AbsoluteSystemPathBuf(unchecked_path.into()))
    }

    /// Takes in a system path of unknown type. If it's absolute, returns the
    /// path, If it's relative, appends it to the base after cleaning it.
    pub fn from_unknown(base: &AbsoluteSystemPath, unknown: impl Into<Utf8PathBuf>) -> Self {
        // we have an absolute system path and an unknown kind of system path.
        let unknown: Utf8PathBuf = unknown.into();
        if unknown.is_absolute() {
            Self(unknown)
        } else {
            Self(
                base.as_path()
                    .join(unknown)
                    .as_std_path()
                    .clean()
                    .try_into()
                    .expect("clean should produce valid UTF-8"),
            )
        }
    }

    pub fn from_cwd(unknown: impl Into<Utf8PathBuf>) -> Result<Self, PathError> {
        let cwd = Self::cwd()?;
        Ok(Self::from_unknown(&cwd, unknown))
    }

    pub fn cwd() -> Result<Self, PathError> {
        // TODO(errors): Unwrap current_dir()
        Ok(Self(Utf8PathBuf::try_from(std::env::current_dir()?)?))
    }

    /// Anchors `path` at `self`.
    ///
    /// # Arguments
    ///
    /// * `path`: The path to be anchored at `self`
    ///
    /// returns: Result<AnchoredSystemPathBuf, PathError>
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    /// use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    /// #[cfg(not(windows))]
    /// {
    ///   let base = AbsoluteSystemPathBuf::new("/Users/user").unwrap();
    ///   let anchored_path = AbsoluteSystemPathBuf::new("/Users/user/Documents").unwrap();
    ///   let anchored_path = base.anchor(&anchored_path).unwrap();
    ///   assert_eq!(anchored_path.as_str(), "Documents");
    /// }
    ///
    /// #[cfg(windows)]
    /// {
    ///   let base = AbsoluteSystemPathBuf::new("C:\\Users\\user").unwrap();
    ///   let anchored_path = AbsoluteSystemPathBuf::new("C:\\Users\\user\\Documents").unwrap();
    ///   let anchored_path = base.anchor(&anchored_path).unwrap();
    ///  assert_eq!(anchored_path.as_str(), "Documents");
    /// }
    /// ```
    pub fn anchor(
        &self,
        path: impl AsRef<AbsoluteSystemPath>,
    ) -> Result<AnchoredSystemPathBuf, PathError> {
        AnchoredSystemPathBuf::new(self, path)
    }

    pub fn as_path(&self) -> &Utf8Path {
        self.0.as_path()
    }

    pub fn components(&self) -> Utf8Components<'_> {
        self.0.components()
    }

    pub fn parent(&self) -> Option<&AbsoluteSystemPath> {
        self.0.parent().map(AbsoluteSystemPath::new_unchecked)
    }

    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.0.starts_with(base.as_ref())
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.0.ends_with(child.as_ref())
    }

    pub fn ensure_dir(&self) -> Result<(), io::Error> {
        if let Some(parent) = self.0.parent() {
            fs::create_dir_all(parent)
        } else {
            Ok(())
        }
    }

    pub fn create_dir_all(&self) -> Result<(), io::Error> {
        fs::create_dir_all(self.0.as_path())
    }

    pub fn remove(&self) -> Result<(), io::Error> {
        fs::remove_file(self.0.as_path())
    }

    pub fn set_readonly(&self) -> Result<(), PathError> {
        let metadata = fs::symlink_metadata(self)?;
        let mut perms = metadata.permissions();
        perms.set_readonly(true);
        fs::set_permissions(self.0.as_path(), perms)?;
        Ok(())
    }

    pub fn is_readonly(&self) -> Result<bool, PathError> {
        Ok(self.0.symlink_metadata()?.permissions().readonly())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.file_name()
    }

    pub fn extension(&self) -> Option<&str> {
        self.0.extension()
    }
}

impl TryFrom<PathBuf> for AbsoluteSystemPathBuf {
    type Error = PathError;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        Self::new(Utf8PathBuf::try_from(path)?)
    }
}

impl TryFrom<&Path> for AbsoluteSystemPathBuf {
    type Error = PathError;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        let utf8_path: &Utf8Path = path.try_into()?;
        Self::new(utf8_path.to_owned())
    }
}

impl TryFrom<&str> for AbsoluteSystemPathBuf {
    type Error = PathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(Utf8PathBuf::from(value))
    }
}

impl From<AbsoluteSystemPathBuf> for PathBuf {
    fn from(path: AbsoluteSystemPathBuf) -> Self {
        path.0.into_std_path_buf()
    }
}

impl fmt::Display for AbsoluteSystemPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl AsRef<Path> for AbsoluteSystemPathBuf {
    fn as_ref(&self) -> &Path {
        self.0.as_std_path()
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use crate::{AbsoluteSystemPathBuf, PathError, RelativeUnixPathBuf};

    #[cfg(not(windows))]
    #[test]
    fn test_absolute_system_path_buf_on_unix() {
        assert!(AbsoluteSystemPathBuf::new("/Users/user").is_ok());
        assert_matches!(
            AbsoluteSystemPathBuf::new("./Users/user/"),
            Err(PathError::NotAbsolute(_))
        );

        assert_matches!(
            AbsoluteSystemPathBuf::new("Users"),
            Err(PathError::NotAbsolute(_))
        );

        let tail = RelativeUnixPathBuf::new("../other").unwrap();

        assert_eq!(
            AbsoluteSystemPathBuf::new("/some/dir")
                .unwrap()
                .join_unix_path(tail),
            AbsoluteSystemPathBuf::new("/some/other").unwrap(),
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_absolute_system_path_buf_on_windows() {
        assert!(AbsoluteSystemPathBuf::new("C:\\Users\\user").is_ok());
        assert_matches!(
            AbsoluteSystemPathBuf::new(".\\Users\\user\\"),
            Err(PathError::NotAbsolute(_))
        );
        assert_matches!(
            AbsoluteSystemPathBuf::new("Users"),
            Err(PathError::NotAbsolute(_))
        );
        assert_matches!(
            AbsoluteSystemPathBuf::new("/Users/home"),
            Err(PathError::NotAbsolute(_))
        );

        let tail = RelativeUnixPathBuf::new("../other").unwrap();

        assert_eq!(
            AbsoluteSystemPathBuf::new("C:\\some\\dir")
                .unwrap()
                .join_unix_path(&tail),
            AbsoluteSystemPathBuf::new("C:\\some\\other").unwrap(),
        );
    }
}
