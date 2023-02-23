use std::{fmt, path::Path};

use camino::Utf8Path;

use crate::{validate_system, AnchoredSystemPathBuf, PathError};

pub struct AnchoredSystemPath(Utf8Path);

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
        self.0.fmt(f)
    }
}

impl AsRef<Path> for AnchoredSystemPath {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl AnchoredSystemPath {
    pub unsafe fn new_unchecked<'a>(path: impl AsRef<Path> + 'a) -> &'a Self {
        let path = path.as_ref();
        unsafe { &*(path as *const Path as *const Self) }
    }

    pub fn new(path: &Path) -> Result<&Self, PathError> {
        if path.is_absolute() {
            return Err(PathError::NotRelative(path.to_string_lossy().to_string()));
        }

        let path_str = path
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(path.to_string_lossy().to_string()))?;

        validate_system(path_str)?;

        Ok(unsafe { AnchoredSystemPath::new_unchecked(path) })
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn parent(&self) -> Option<&AnchoredSystemPath> {
        self.0
            .parent()
            .map(|path| unsafe { AnchoredSystemPath::new_unchecked(path) })
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_std_path()
    }
}
