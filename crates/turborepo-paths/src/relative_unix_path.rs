use std::{
    fmt,
    fmt::{Display, Formatter},
};

use camino::{Utf8Path, Utf8PathBuf};

use crate::{AnchoredSystemPathBuf, PathError, RelativeUnixPathBuf};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    pub(crate) fn to_system_path_buf(&self) -> Utf8PathBuf {
        #[cfg(unix)]
        {
            // On unix, unix paths are already system paths. Copy the string
            // but skip validation.
            Utf8PathBuf::from(&self.0)
        }

        #[cfg(windows)]
        {
            let system_path_string = self.0.replace('/', "\\");
            Utf8PathBuf::from(system_path_string)
        }
    }

    pub fn to_anchored_system_path_buf(&self) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf(self.to_system_path_buf())
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::AnchoredSystemPath;

    #[test]
    fn test_to_anchored_system_path_buf() {
        let path = RelativeUnixPath::new("foo/bar/baz").unwrap();
        let expected = AnchoredSystemPath::new(if cfg!(windows) {
            // Unix path separators should be converted
            "foo\\bar\\baz"
        } else {
            // Unix paths already have correct separators
            "foo/bar/baz"
        })
        .unwrap();
        assert_eq!(&*path.to_anchored_system_path_buf(), expected);
    }
}
