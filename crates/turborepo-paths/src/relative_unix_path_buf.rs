use std::{
    borrow::Borrow,
    fmt,
    fmt::{Debug, Display, Formatter},
    io::Write,
    ops::Deref,
};

use crate::{PathError, RelativeUnixPath};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RelativeUnixPathBuf(pub(crate) String);

impl Display for RelativeUnixPathBuf {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl RelativeUnixPathBuf {
    pub fn new(path: impl Into<String>) -> Result<Self, PathError> {
        let path_string = path.into();
        if path_string.starts_with('/') {
            return Err(PathError::NotRelative(path_string));
        }
        Ok(Self(path_string))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    // write_escaped_bytes writes this path to the given writer in the form
    // "<escaped path>", where escaped_path is the path with '"' and '\n'
    // characters escaped with '\'.
    pub fn write_escaped_bytes<W: Write>(&self, writer: &mut W) -> Result<(), PathError> {
        writer.write_all(&[b'\"'])?;
        // i is our pointer into self.0, and to_escape_index is a pointer to the next
        // byte to be escaped. Each time we find a byte to be escaped, we write
        // out everything from i to to_escape_index, then the escape byte, '\\',
        // then the byte-to-be-escaped. Finally we set i to 1 + to_escape_index
        // to move our pointer past the byte we just escaped.
        let mut i: usize = 0;
        for (to_escaped_index, byte) in self
            .0
            .bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\"' || *byte == b'\n')
        {
            writer.write_all(self.0[i..to_escaped_index].as_bytes())?;
            writer.write_all(&[b'\\', byte])?;
            i = to_escaped_index + 1;
        }
        if i < self.0.len() {
            writer.write_all(self.0[i..].as_bytes())?;
        }
        writer.write_all(&[b'\"'])?;
        Ok(())
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

impl Debug for RelativeUnixPathBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufWriter;
    #[cfg(windows)]
    use std::path::Path;

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
    fn test_write_escaped() {
        let input = "\"quote\"\nnewline\n";
        let expected = b"\"\\\"quote\\\"\\\nnewline\\\n\"";
        let mut buffer = Vec::new();
        {
            let mut writer = BufWriter::new(&mut buffer);
            let path = RelativeUnixPathBuf::new(input).unwrap();
            path.write_escaped_bytes(&mut writer).unwrap();
        }
        assert_eq!(buffer.as_slice(), expected);
    }

    #[test]
    fn test_relative_unix_path_buf_errors() {
        assert!(RelativeUnixPathBuf::new("/foo/bar").is_err());
        // Note: this shouldn't be an error, this is a valid relative unix path
        // #[cfg(windows)]
        // assert!(RelativeUnixPathBuf::new(PathBuf::from("C:\\foo\\bar")).
        // is_err());
    }
}
