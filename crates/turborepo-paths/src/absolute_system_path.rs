#[cfg(not(windows))]
use std::os::unix::fs::symlink as symlink_file;
#[cfg(not(windows))]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file};
use std::{
    fmt, fs,
    fs::{File, Metadata},
    io,
    path::Path,
};

use camino::{Utf8Components, Utf8Path, Utf8PathBuf};
use path_clean::PathClean;

use crate::{
    is_not_system, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, IntoSystem, PathError,
    RelativeUnixPath,
};

pub struct AbsoluteSystemPath(Utf8Path);

impl ToOwned for AbsoluteSystemPath {
    type Owned = AbsoluteSystemPathBuf;

    fn to_owned(&self) -> Self::Owned {
        AbsoluteSystemPathBuf(self.0.to_owned())
    }
}

impl AsRef<AbsoluteSystemPath> for AbsoluteSystemPath {
    fn as_ref(&self) -> &AbsoluteSystemPath {
        self
    }
}

impl fmt::Display for AbsoluteSystemPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl AsRef<Path> for AbsoluteSystemPath {
    fn as_ref(&self) -> &Path {
        self.0.as_std_path()
    }
}

impl AbsoluteSystemPath {
    /// Creates a path that is known to be absolute and a system path.
    /// If either of these conditions are not met, we error.
    /// Does *not* do automatic conversion like `AbsoluteSystemPathBuf::new`
    /// does
    ///
    /// # Arguments
    ///
    /// * `value`: The path to convert to an absolute system path
    ///
    /// returns: Result<&AbsoluteSystemPath, PathError>
    ///
    /// # Examples
    ///
    /// ```
    /// use turbopath::AbsoluteSystemPath;
    /// #[cfg(unix)]
    /// {
    ///   assert!(AbsoluteSystemPath::new("/foo/bar").is_ok());
    ///   assert!(AbsoluteSystemPath::new("foo/bar").is_err());
    ///   assert!(AbsoluteSystemPath::new("C:\\foo\\bar").is_err());
    /// }
    ///
    /// #[cfg(windows)]
    /// {
    ///   assert!(AbsoluteSystemPath::new("C:\\foo\\bar").is_ok());
    ///   assert!(AbsoluteSystemPath::new("foo\\bar").is_err());
    ///   assert!(AbsoluteSystemPath::new("/foo/bar").is_err());
    /// }
    /// ```
    pub fn new<P: AsRef<str> + ?Sized>(value: &P) -> Result<&Self, PathError> {
        let path = value.as_ref();
        if Path::new(path).is_relative() {
            return Err(PathError::NotAbsolute(path.to_owned()));
        }

        if is_not_system(path) {
            return Err(PathError::NotSystem(path.to_owned()));
        }

        Ok(Self::new_unchecked(path))
    }

    pub fn from_std_path(path: &Path) -> Result<&Self, PathError> {
        let path_str = path
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(path.to_string_lossy().to_string()))?;

        if is_not_system(path_str) {
            return Err(PathError::NotSystem(path_str.to_owned()));
        }

        Self::new(path_str)
    }

    pub(crate) fn new_unchecked<'a>(path: impl AsRef<str> + 'a) -> &'a Self {
        let path = Utf8Path::new(path.as_ref());
        unsafe { &*(path as *const Utf8Path as *const Self) }
    }

    pub fn as_path(&self) -> &Utf8Path {
        &self.0
    }

    pub fn as_std_path(&self) -> &Path {
        self.0.as_std_path()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_str().as_bytes()
    }

    pub fn ancestors(&self) -> impl Iterator<Item = &AbsoluteSystemPath> {
        self.0.ancestors().map(Self::new_unchecked)
    }

    // intended for joining literals or obviously single-token strings
    pub fn join_component(&self, segment: &str) -> AbsoluteSystemPathBuf {
        debug_assert!(!segment.contains(std::path::MAIN_SEPARATOR));
        AbsoluteSystemPathBuf(
            self.0
                .join(segment)
                .as_std_path()
                .clean()
                .try_into()
                .unwrap(),
        )
    }

    // intended for joining a path composed of literals
    pub fn join_components(&self, segments: &[&str]) -> AbsoluteSystemPathBuf {
        debug_assert!(!segments
            .iter()
            .any(|segment| segment.contains(std::path::MAIN_SEPARATOR)));
        AbsoluteSystemPathBuf(
            self.0
                .join(segments.join(std::path::MAIN_SEPARATOR_STR))
                .as_std_path()
                .clean()
                .try_into()
                .unwrap(),
        )
    }

    pub fn join_unix_path(
        &self,
        unix_path: impl AsRef<RelativeUnixPath>,
    ) -> Result<AbsoluteSystemPathBuf, PathError> {
        let tail = unix_path.as_ref().to_system_path_buf()?;
        Ok(AbsoluteSystemPathBuf(
            self.0.join(tail).as_std_path().clean().try_into()?,
        ))
    }

    pub fn anchor(&self, path: &AbsoluteSystemPath) -> Result<AnchoredSystemPathBuf, PathError> {
        AnchoredSystemPathBuf::new(self, path)
    }

    pub fn ensure_dir(&self) -> Result<(), io::Error> {
        if let Some(parent) = self.0.parent() {
            fs::create_dir_all(parent)
        } else {
            Ok(())
        }
    }

    pub fn open(&self) -> Result<File, io::Error> {
        File::open(self.0.as_std_path())
    }

    pub fn symlink_to_file<P: AsRef<str>>(&self, to: P) -> Result<(), PathError> {
        let system_path = to.as_ref();
        let system_path = system_path.into_system();
        symlink_file(system_path, &self.0)?;
        Ok(())
    }

    pub fn symlink_to_dir<P: AsRef<str>>(&self, to: P) -> Result<(), PathError> {
        let system_path = to.as_ref();

        let system_path = system_path.into_system();
        symlink_dir(system_path, &self.0)?;

        Ok(())
    }

    pub fn resolve(&self, path: &AnchoredSystemPathBuf) -> AbsoluteSystemPathBuf {
        let path = self.0.join(path);
        AbsoluteSystemPathBuf(path)
    }

    // note that this is *not* lstat. If this is a symlink, it
    // will return metadata for the target.
    pub fn stat(&self) -> Result<Metadata, PathError> {
        Ok(fs::metadata(&self.0)?)
    }

    // The equivalent of lstat. Returns the metadata for this file,
    // even if it is a symlink
    pub fn symlink_metadata(&self) -> Result<Metadata, PathError> {
        Ok(fs::symlink_metadata(&self.0)?)
    }

    pub fn read_link(&self) -> Result<Utf8PathBuf, io::Error> {
        self.0.read_link_utf8()
    }

    pub fn remove_file(&self) -> Result<(), io::Error> {
        fs::remove_file(&self.0)
    }

    pub fn components(&self) -> Utf8Components<'_> {
        self.0.components()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_create_absolute_path() -> Result<()> {
        #[cfg(unix)]
        {
            let absolute_path = AbsoluteSystemPath::new("/foo/bar")?;
            assert_eq!(absolute_path.to_string(), "/foo/bar");
        }

        #[cfg(windows)]
        {
            let absolute_path = AbsoluteSystemPath::new(r"C:\foo\bar")?;
            assert_eq!(absolute_path.to_string(), r"C:\foo\bar");
        }

        Ok(())
    }

    #[test]
    fn test_resolve_empty() {
        let root = AbsoluteSystemPathBuf::cwd().unwrap();
        let empty = AnchoredSystemPathBuf::from_raw("").unwrap();
        let result = root.resolve(&empty);
        assert_eq!(result, root);
    }
}
