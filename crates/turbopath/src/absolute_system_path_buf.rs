use std::{
    fmt,
    path::{Components, Path, PathBuf},
};

use crate::{AnchoredSystemPathBuf, IntoSystem, PathValidationError};

pub struct AbsoluteSystemPathBuf(PathBuf);

impl AbsoluteSystemPathBuf {
    pub fn new(unchecked_path: impl Into<PathBuf>) -> Result<Self, PathValidationError> {
        let unchecked_path = unchecked_path.into();
        if !unchecked_path.is_absolute() {
            return Err(PathValidationError::NotAbsolute(unchecked_path));
        }

        let system_path = unchecked_path.into_system()?;
        Ok(AbsoluteSystemPathBuf(system_path))
    }

    pub fn new_unchecked(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        AbsoluteSystemPathBuf(path)
    }

    /// Resolves `path` with `self` as anchor.
    ///
    /// # Arguments
    ///
    /// * `path`: The path to be anchored at `self`
    ///
    /// returns: AbsoluteSystemPathBuf
    ///
    /// # Examples
    ///
    /// ```
    /// ```
    pub fn resolve(&self, path: &AnchoredSystemPathBuf) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf(self.0.join(path.as_path()))
    }

    /// Anchors `path` at `self`.
    ///
    /// # Arguments
    ///
    /// * `root`:
    ///
    /// returns: Result<AnchoredSystemPathBuf, PathValidationError>
    ///
    /// # Examples
    ///
    /// ```
    /// ```
    pub fn anchor(
        &self,
        path: &AbsoluteSystemPathBuf,
    ) -> Result<AnchoredSystemPathBuf, PathValidationError> {
        AnchoredSystemPathBuf::strip_root(&self, path)
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }

    pub fn components(&self) -> Components<'_> {
        self.0.components()
    }

    pub fn parent(&self) -> Option<Self> {
        self.0
            .parent()
            .map(|p| AbsoluteSystemPathBuf(p.to_path_buf()))
    }

    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.0.starts_with(base.as_ref())
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.0.ends_with(child.as_ref())
    }

    pub fn join<P: AsRef<Path>>(&self, path: P) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf(self.0.join(path))
    }

    pub fn to_str(&self) -> Result<&str, PathValidationError> {
        self.0.to_str().ok_or(PathValidationError::InvalidUnicode)
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.file_name().and_then(|s| s.to_str())
    }

    pub fn extension(&self) -> Option<&str> {
        self.0.extension().and_then(|s| s.to_str())
    }
}

impl fmt::Display for AbsoluteSystemPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

impl fmt::Debug for AbsoluteSystemPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq for AbsoluteSystemPathBuf {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
