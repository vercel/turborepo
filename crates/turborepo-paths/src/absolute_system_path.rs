#[cfg(not(windows))]
use std::os::unix::fs::symlink as symlink_file;
#[cfg(not(windows))]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file};
use std::{
    borrow::Cow,
    ffi::OsStr,
    fmt, fs,
    fs::{File, Metadata, OpenOptions},
    io,
    path::{Components, Path, PathBuf},
};

use path_clean::PathClean;
use path_slash::CowExt;

use crate::{
    AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, IntoSystem, PathError,
    RelativeUnixPath,
};

#[derive(Debug)]
pub struct AbsoluteSystemPath(Path);

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
        self.0.display().fmt(f)
    }
}

impl AsRef<Path> for AbsoluteSystemPath {
    fn as_ref(&self) -> &Path {
        &self.0
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
    pub fn new<P: AsRef<Path> + ?Sized>(value: &P) -> Result<&Self, PathError> {
        let path = value.as_ref();
        if path.is_relative() {
            return Err(PathError::NotAbsolute(path.to_owned()));
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(path.to_string_lossy().to_string()))?;

        let system_path = Cow::from_slash(path_str);

        match system_path {
            Cow::Owned(path) => Err(PathError::NotSystem(path.to_string_lossy().to_string())),
            Cow::Borrowed(path) => {
                let path = Path::new(path);
                // copied from stdlib path.rs: relies on the representation of
                // AbsoluteSystemPath being just a Path, the same way Path relies on
                // just being an OsStr
                let absolute_system_path = unsafe { &*(path as *const Path as *const Self) };
                Ok(absolute_system_path)
            }
        }
    }

    pub(crate) unsafe fn new_unchecked(path: &Path) -> &Self {
        &*(path as *const Path as *const Self)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn ancestors(&self) -> impl Iterator<Item = &AbsoluteSystemPath> {
        self.0
            .ancestors()
            .map(|ancestor| unsafe { Self::new_unchecked(ancestor) })
    }

    pub fn create_dir_all(&self) -> Result<(), io::Error> {
        fs::create_dir_all(&self.0)
    }

    pub fn extension(&self) -> Option<&OsStr> {
        self.0.extension()
    }

    // intended for joining literals or obviously single-token strings
    pub fn join_component(&self, segment: &str) -> AbsoluteSystemPathBuf {
        debug_assert!(!segment.contains(std::path::MAIN_SEPARATOR));
        AbsoluteSystemPathBuf(self.0.join(segment).clean())
    }

    // intended for joining a path composed of literals
    pub fn join_components(&self, segments: &[&str]) -> AbsoluteSystemPathBuf {
        debug_assert!(!segments
            .iter()
            .any(|segment| segment.contains(std::path::MAIN_SEPARATOR)));
        AbsoluteSystemPathBuf(
            self.0
                .join(segments.join(std::path::MAIN_SEPARATOR_STR))
                .clean(),
        )
    }

    pub fn join_unix_path(
        &self,
        unix_path: impl AsRef<RelativeUnixPath>,
    ) -> Result<AbsoluteSystemPathBuf, PathError> {
        let tail = unix_path.as_ref().to_system_path_buf()?;
        Ok(AbsoluteSystemPathBuf(self.0.join(tail.as_path()).clean()))
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

    pub fn symlink_to_file<P: AsRef<Path>>(&self, to: P) -> Result<(), PathError> {
        let system_path = to.as_ref();
        let system_path = system_path.into_system()?;
        symlink_file(system_path, &self.0)?;
        Ok(())
    }

    pub fn symlink_to_dir<P: AsRef<Path>>(&self, to: P) -> Result<(), PathError> {
        let system_path = to.as_ref();
        let system_path = system_path.into_system()?;
        symlink_dir(system_path, &self.0)?;
        Ok(())
    }

    pub fn resolve(&self, path: impl AsRef<AnchoredSystemPath>) -> AbsoluteSystemPathBuf {
        let path = self.0.join(path.as_ref());
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

    pub fn read_link(&self) -> Result<PathBuf, io::Error> {
        fs::read_link(&self.0)
    }

    pub fn remove_file(&self) -> Result<(), io::Error> {
        fs::remove_file(&self.0)
    }

    pub fn components(&self) -> Components<'_> {
        self.0.components()
    }

    pub fn parent(&self) -> Option<&AbsoluteSystemPath> {
        self.0
            .parent()
            .map(|p| unsafe { AbsoluteSystemPath::new_unchecked(p) })
    }

    /// Opens file and sets the `FILE_FLAG_SEQUENTIAL_SCAN` flag on Windows to
    /// help with performance
    pub fn open(&self) -> Result<File, io::Error> {
        let mut options = OpenOptions::new();
        options.read(true);

        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            use crate::FILE_FLAG_SEQUENTIAL_SCAN;

            options.custom_flags(FILE_FLAG_SEQUENTIAL_SCAN);
        }

        options.open(&self.0)
    }

    pub fn open_with_options(&self, open_options: OpenOptions) -> Result<File, io::Error> {
        open_options.open(&self.0)
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
