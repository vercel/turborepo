use std::{
    fmt,
    path::{Components, Path, PathBuf},
};

use crate::{relative_system_path::RelativeSystemPath, IntoSystem, PathValidationError};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RelativeSystemPathBuf(PathBuf);

impl RelativeSystemPathBuf {
    pub fn new(unchecked_path: PathBuf) -> Result<Self, PathValidationError> {
        if unchecked_path.is_absolute() {
            return Err(PathValidationError::NotRelative(unchecked_path));
        }

        let system_path = unchecked_path.into_system()?;
        Ok(RelativeSystemPathBuf(system_path))
    }

    pub fn new_unchecked(path: PathBuf) -> Self {
        RelativeSystemPathBuf(path)
    }

    pub fn as_relative_path(&self) -> RelativeSystemPath {
        RelativeSystemPath::new_unchecked(&self.0)
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
            .map(|p| RelativeSystemPathBuf(p.to_path_buf()))
    }

    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.0.starts_with(base.as_ref())
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.0.ends_with(child.as_ref())
    }

    pub fn join<P: AsRef<Path>>(&self, path: P) -> RelativeSystemPathBuf {
        RelativeSystemPathBuf(self.0.join(path))
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

impl fmt::Display for RelativeSystemPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}
