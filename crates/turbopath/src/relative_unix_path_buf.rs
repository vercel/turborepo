use std::path::{Components, Path, PathBuf};

use path_slash::PathBufExt;

use crate::PathValidationError;

pub struct RelativeUnixPathBuf(PathBuf);

impl RelativeUnixPathBuf {
    pub fn new(path: PathBuf) -> Result<Self, PathValidationError> {
        if path.is_absolute() {
            return Err(PathValidationError::NotRelative);
        }

        let path = path.to_slash().ok_or(PathValidationError::NonUtf8)?;
        Ok(RelativeUnixPathBuf(PathBuf::from(path.to_string())))
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
