use std::{
    fmt,
    path::{Components, Path, PathBuf},
};

use crate::{
    absolute_system_path::AbsoluteSystemPath, AnchoredSystemPathBuf, IntoSystem,
    PathValidationError,
};

pub struct AbsoluteSystemPathBuf(PathBuf);

impl AbsoluteSystemPathBuf {
    pub fn new(unchecked_path: PathBuf) -> Result<Self, PathValidationError> {
        if !unchecked_path.is_absolute() {
            return Err(PathValidationError::NotAbsolute(unchecked_path));
        }

        let system_path = unchecked_path.into_system()?;
        Ok(AbsoluteSystemPathBuf(system_path))
    }

    pub fn anchor_at(
        &self,
        root: &AbsoluteSystemPath,
    ) -> Result<AnchoredSystemPathBuf, PathValidationError> {
        AnchoredSystemPathBuf::strip_root(root, &self.as_absolute_path())
    }

    pub fn new_unchecked(path: PathBuf) -> Self {
        AbsoluteSystemPathBuf(path)
    }

    pub fn as_absolute_path(&self) -> AbsoluteSystemPath {
        AbsoluteSystemPath::new_unchecked(self.0.as_path())
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
