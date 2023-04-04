use std::path::{Path, PathBuf};

use crate::{AbsoluteSystemPathBuf, IntoSystem, PathValidationError};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AnchoredSystemPathBuf(PathBuf);

impl TryFrom<&Path> for AnchoredSystemPathBuf {
    type Error = PathValidationError;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        if path.is_absolute() {
            return Err(PathValidationError::NotRelative(path.to_path_buf()));
        }

        Ok(AnchoredSystemPathBuf(path.into_system()?))
    }
}

impl AnchoredSystemPathBuf {
    pub fn strip_root(
        root: &AbsoluteSystemPathBuf,
        path: &AbsoluteSystemPathBuf,
    ) -> Result<Self, PathValidationError> {
        let stripped_path = path
            .as_path()
            .strip_prefix(root.as_path())
            .map_err(|_| PathValidationError::NotParent(root.to_string(), path.to_string()))?
            .to_path_buf();

        Ok(AnchoredSystemPathBuf(stripped_path))
    }

    pub fn new_unchecked(path: impl Into<PathBuf>) -> Self {
        AnchoredSystemPathBuf(path.into())
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }

    pub fn to_str(&self) -> Result<&str, PathValidationError> {
        self.0
            .to_str()
            .ok_or_else(|| PathValidationError::InvalidUnicode)
    }
}
