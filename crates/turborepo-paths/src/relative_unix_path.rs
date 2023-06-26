use std::{
    fmt,
    fmt::{Display, Formatter},
};

use camino::{Utf8Path, Utf8PathBuf};

use crate::{PathError, RelativeUnixPathBuf};

#[repr(transparent)]
pub struct RelativeUnixPath(str);

impl Display for RelativeUnixPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

impl RelativeUnixPath {
    pub fn new<'a, P: AsRef<str> + 'a>(value: P) -> Result<&'a Self, PathError> {
        let path = value.as_ref();
        if path.starts_with('/') {
            return Err(PathError::NotRelative(path.to_string()));
        }
        // copied from stdlib path.rs: relies on the representation of
        // RelativeUnixPath being just a str, the same way Path relies on
        // just being an OsStr
        Ok(unsafe { &*(path as *const str as *const Self) })
    }

    pub(crate) fn to_system_path_buf(&self) -> Result<Utf8PathBuf, PathError> {
        #[cfg(unix)]
        {
            // On unix, unix paths are already system paths. Copy the string
            // but skip validation.
            Ok(Utf8PathBuf::from(&self.0))
        }

        #[cfg(windows)]
        {
            let system_path_string = self.0.replace('/', "\\");
            Ok(Utf8PathBuf::from(system_path_string))
        }
    }

    pub fn to_owned(&self) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf(self.0.to_owned())
    }

    pub fn strip_prefix(
        &self,
        prefix: impl AsRef<RelativeUnixPath>,
    ) -> Result<&RelativeUnixPath, PathError> {
        let stripped_path = self
            .0
            .strip_prefix(&prefix.as_ref().0)
            .ok_or_else(|| PathError::NotParent(prefix.as_ref().to_string(), self.to_string()))?;

        // Remove leading '/' if present
        let stripped_path = stripped_path.strip_prefix('/').unwrap_or(stripped_path);

        Ok(unsafe { &*(stripped_path as *const str as *const Self) })
    }

    // NOTE: This only applies to full path components. If you
    // want to check a file extension, use `RelativeUnixPathBuf::extension`.
    pub fn ends_with(&self, suffix: impl AsRef<str>) -> bool {
        self.0.ends_with(suffix.as_ref())
    }

    pub fn extension(&self) -> Option<&str> {
        Utf8Path::new(&self.0).extension()
    }
}

impl AsRef<RelativeUnixPath> for RelativeUnixPath {
    fn as_ref(&self) -> &RelativeUnixPath {
        self
    }
}
