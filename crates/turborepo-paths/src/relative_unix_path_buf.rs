use std::{
    borrow::Borrow,
    fmt,
    fmt::{Display, Formatter},
    ops::Deref,
};

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{PathError, RelativeUnixPath};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
// This is necessary to perform validation on the string during deserialization
#[serde(try_from = "String", into = "String")]
pub struct RelativeUnixPathBuf(pub(crate) String);

#[cfg(feature = "biome")]
mod biome {
    use biome_deserialize::{Deserializable, DeserializableValue, DeserializationDiagnostic};
    use turborepo_unescape::UnescapedString;

    use crate::RelativeUnixPathBuf;

    impl Deserializable for RelativeUnixPathBuf {
        fn deserialize(
            value: &impl DeserializableValue,
            name: &str,
            diagnostics: &mut Vec<DeserializationDiagnostic>,
        ) -> Option<Self> {
            let path: String = UnescapedString::deserialize(value, name, diagnostics)?.into();
            match Self::new(path) {
                Ok(path) => Some(path),
                Err(e) => {
                    diagnostics.push(
                        DeserializationDiagnostic::new(e.to_string()).with_range(value.range()),
                    );
                    None
                }
            }
        }
    }
}

impl Display for RelativeUnixPathBuf {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl RelativeUnixPathBuf {
    pub fn new(path: impl Into<String>) -> Result<Self, PathError> {
        let path_string = path.into();
        if path_string.starts_with('/') || Utf8Path::new(&path_string).is_absolute() {
            return Err(PathError::NotRelative(path_string));
        }

        Ok(Self(path_string))
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    pub fn make_canonical_for_tar(&mut self, is_dir: bool) {
        if is_dir && !self.0.ends_with('/') {
            self.0.push('/');
        }
    }

    pub fn strip_prefix(&self, prefix: &RelativeUnixPathBuf) -> Result<Self, PathError> {
        let prefix_len = prefix.0.len();
        if prefix_len == 0 {
            return Ok(self.clone());
        }
        if !self.0.starts_with(&prefix.0) {
            return Err(PathError::NotParent(
                prefix.0.to_string(),
                self.0.to_string(),
            ));
        }

        // Handle the case where we are stripping the entire contents of this path
        if self.0.len() == prefix.0.len() {
            return Self::new("");
        }

        // We now know that this path starts with the prefix, and that this path's
        // length is greater than the prefix's length
        if self.0.as_bytes()[prefix_len] != b'/' {
            let prefix_str = prefix.0.clone();
            let this = self.0.clone();
            return Err(PathError::PrefixError(prefix_str, this));
        }

        let tail_slice = &self.0[(prefix_len + 1)..];
        Self::new(tail_slice)
    }
}

pub trait RelativeUnixPathBufTestExt {
    fn join(&self, tail: &RelativeUnixPathBuf) -> Self;
}

impl RelativeUnixPathBufTestExt for RelativeUnixPathBuf {
    // Marked as test-only because it doesn't automatically clean the resulting
    // path. *If* we end up needing or wanting this method outside of tests, we
    // will need to implement .clean() for the result.
    fn join(&self, tail: &RelativeUnixPathBuf) -> Self {
        if self.0.is_empty() {
            return tail.clone();
        }
        let mut joined = self.0.clone();
        joined.push('/');
        joined.push_str(&tail.0);
        Self(joined)
    }
}

impl Borrow<RelativeUnixPath> for RelativeUnixPathBuf {
    fn borrow(&self) -> &RelativeUnixPath {
        let inner: &str = self.0.borrow();
        unsafe { &*(inner as *const str as *const RelativeUnixPath) }
    }
}

impl AsRef<RelativeUnixPath> for RelativeUnixPathBuf {
    fn as_ref(&self) -> &RelativeUnixPath {
        self.borrow()
    }
}

impl Deref for RelativeUnixPathBuf {
    type Target = RelativeUnixPath;

    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl TryFrom<String> for RelativeUnixPathBuf {
    type Error = PathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

// From<String> should not be implemented for RelativeUnixPathBuf as validation
// may fail
#[allow(clippy::from_over_into)]
impl Into<String> for RelativeUnixPathBuf {
    fn into(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_relative_unix_path_buf() {
        let path = RelativeUnixPathBuf::new("foo/bar").unwrap();
        assert_eq!(path.as_str(), "foo/bar");
    }

    #[test]
    fn test_relative_unix_path_buf_with_extension() {
        let path = RelativeUnixPathBuf::new("foo/bar.txt").unwrap();
        assert_eq!(path.as_str(), "foo/bar.txt");
    }

    #[test]
    fn test_join() {
        let head = RelativeUnixPathBuf::new("some/path").unwrap();
        let tail = RelativeUnixPathBuf::new("child/leaf").unwrap();
        let combined = head.join(&tail);
        assert_eq!(combined.as_str(), "some/path/child/leaf");
    }

    #[test]
    fn test_strip_prefix() {
        let combined = RelativeUnixPathBuf::new("some/path/child/leaf").unwrap();
        let head = RelativeUnixPathBuf::new("some/path").unwrap();
        let expected = RelativeUnixPathBuf::new("child/leaf").unwrap();
        let tail = combined.strip_prefix(&head).unwrap();
        assert_eq!(tail, expected);
    }

    #[test]
    fn test_strip_entire_contents() {
        let combined = RelativeUnixPathBuf::new("some/path").unwrap();
        let head = combined.clone();
        let expected = RelativeUnixPathBuf::new("").unwrap();
        let tail = combined.strip_prefix(&head).unwrap();
        assert_eq!(tail, expected);
    }

    #[test]
    fn test_strip_empty_prefix() {
        let combined = RelativeUnixPathBuf::new("some/path").unwrap();
        let tail = combined
            .strip_prefix(&RelativeUnixPathBuf::new("").unwrap())
            .unwrap();
        assert_eq!(tail, combined);
    }

    #[test]
    fn test_relative_unix_path_buf_errors() {
        assert!(RelativeUnixPathBuf::new("/foo/bar").is_err());
        // Note: this shouldn't be an error, this is a valid relative unix path
        // #[cfg(windows)]
        // assert!(RelativeUnixPathBuf::new(PathBuf::from("C:\\foo\\bar")).
        // is_err());
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct TestSchema {
        field: RelativeUnixPathBuf,
    }

    #[test]
    fn test_roundtrip() {
        let path = "relative/unix/path\\evil";
        let value = json!({ "field": path });
        let foo: TestSchema = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(foo.field.deref(), RelativeUnixPath::new(path).unwrap());
        assert_eq!(serde_json::to_value(foo).unwrap(), value);
    }

    #[test]
    fn test_deserialization_fails_on_absolute() {
        let foo: Result<TestSchema, _> = serde_json::from_value(json!({"field": "/absolute/path"}));
        let Err(e) = foo else {
            panic!("expected absolute path deserialization to fail")
        };
        assert_eq!(e.to_string(), "Path is not relative: /absolute/path");
    }
}
