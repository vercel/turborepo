use std::{
    fmt,
    path::{Component, Path},
};

use crate::{AnchoredSystemPathBuf, PathError};

pub struct AnchoredSystemPath(Path);

impl ToOwned for AnchoredSystemPath {
    type Owned = AnchoredSystemPathBuf;

    fn to_owned(&self) -> Self::Owned {
        AnchoredSystemPathBuf(self.0.to_owned())
    }
}

impl AsRef<AnchoredSystemPath> for AnchoredSystemPath {
    fn as_ref(&self) -> &AnchoredSystemPath {
        self
    }
}

impl fmt::Display for AnchoredSystemPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

impl AsRef<Path> for AnchoredSystemPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AnchoredSystemPath {
    pub(crate) unsafe fn new_unchecked<'a>(path: impl AsRef<Path> + 'a) -> &'a Self {
        let path = path.as_ref();
        unsafe { &*(path as *const Path as *const Self) }
    }

    pub fn new<'a, T: AsRef<Path> + 'a>(path: T) -> Result<&'a Self, PathError> {
        let path_ref = path.as_ref();
        if path_ref.is_absolute() {
            return Err(PathError::NotRelative(
                path_ref.to_string_lossy().to_string(),
            ));
        }

        #[cfg(windows)]
        {
            let path_str = path_ref
                .to_str()
                .ok_or_else(|| PathError::InvalidUnicode(path_ref.to_string_lossy().to_string()))?;
            if path_str.contains('/') {
                return Err(PathError::NotSystem(path_str.to_string()));
            }
        }

        Ok(unsafe { &*(path_ref as *const Path as *const Self) })
    }

    pub fn to_str(&self) -> Result<&str, PathError> {
        self.0
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(self.0.to_string_lossy().to_string()))
    }

    pub fn parent(&self) -> Option<&AnchoredSystemPath> {
        self.0
            .parent()
            .map(|path| unsafe { AnchoredSystemPath::new_unchecked(path) })
    }

    pub fn components(&self) -> impl Iterator<Item = Component> {
        self.0.components()
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}
