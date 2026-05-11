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
    pub fn new<P: AsRef<str> + ?Sized>(value: &P) -> Result<&Self, PathError> {
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

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_owned(&self) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf(self.0.to_owned())
    }

    pub fn strip_prefix(
        &self,
        prefix: impl AsRef<RelativeUnixPath>,
    ) -> Result<&RelativeUnixPath, PathError> {
        let prefix = prefix.as_ref();
        let prefix_len = prefix.0.len();
        if prefix_len == 0 {
            return Ok(self);
        }

        if !self.0.starts_with(&prefix.0) {
            return Err(PathError::NotParent(prefix.to_string(), self.to_string()));
        }

        if self.0.len() == prefix_len {
            let empty = "";
            return Ok(unsafe { &*(empty as *const str as *const Self) });
        }

        if self.0.as_bytes()[prefix_len] != b'/' {
            return Err(PathError::PrefixError(prefix.to_string(), self.to_string()));
        }

        let stripped_path = &self.0[(prefix_len + 1)..];

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

    pub fn join_component(&self, segment: &str) -> RelativeUnixPathBuf {
        debug_assert!(!segment.contains('/'));
        RelativeUnixPathBuf(format!("{}/{}", &self.0, segment))
    }
}

impl AsRef<RelativeUnixPath> for RelativeUnixPath {
    fn as_ref(&self) -> &RelativeUnixPath {
        self
    }
}

impl<'a> From<&'a RelativeUnixPath> for wax::CandidatePath<'a> {
    fn from(path: &'a RelativeUnixPath) -> Self {
        path.0.into()
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

    #[test]
    fn test_strip_prefix_rejects_partial_component_match() {
        let path = RelativeUnixPath::new("foobar/baz").unwrap();
        let prefix = RelativeUnixPath::new("foo").unwrap();

        assert!(path.strip_prefix(prefix).is_err());
    }

    #[test]
    fn test_strip_prefix_accepts_component_boundary_match() {
        let path = RelativeUnixPath::new("foo/bar/baz").unwrap();
        let prefix = RelativeUnixPath::new("foo").unwrap();

        let stripped = path.strip_prefix(prefix).unwrap();

        assert_eq!(stripped.as_str(), "bar/baz");
    }
}
