use std::{
    fmt,
    path::{Components, Path},
};

use crate::{relative_system_path_buf::RelativeSystemPathBuf, PathValidationError};

pub struct RelativeSystemPath<'a>(&'a Path);

impl<'a> RelativeSystemPath<'a> {
    fn new_unchecked(path: &'a Path) -> Self {
        RelativeSystemPath(path)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn components(&self) -> Components<'_> {
        self.0.components()
    }

    pub fn parent(&self) -> Option<Self> {
        self.0.parent().map(RelativeSystemPath::new_unchecked)
    }

    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        self.0.starts_with(base.as_ref())
    }

    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        self.0.ends_with(child.as_ref())
    }

    pub fn join<P: AsRef<RelativeSystemPath<'a>>>(&self, path: P) -> RelativeSystemPathBuf {
        RelativeSystemPathBuf::new_unchecked(self.0.join(path.as_ref().as_path()))
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

impl<'a> fmt::Display for RelativeSystemPath<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

impl<'a> fmt::Debug for RelativeSystemPath<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a> PartialEq for RelativeSystemPath<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
