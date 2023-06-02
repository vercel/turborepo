use std::{borrow::Borrow, fmt::Debug, io::Write, ops::Deref};

use bstr::{BStr, BString, ByteSlice};

use crate::{AnchoredSystemPath, AnchoredUnixTarPathBuf, PathError, RelativeUnixPath};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RelativeUnixPathBuf(pub(crate) BString);

impl RelativeUnixPathBuf {
    pub fn new(path: impl Into<Vec<u8>>) -> Result<Self, PathError> {
        let bytes: Vec<u8> = path.into();
        if bytes.first() == Some(&b'/') {
            return Err(PathError::not_relative_error(&bytes));
        }
        Ok(Self(BString::new(bytes)))
    }

    pub fn as_str(&self) -> Result<&str, PathError> {
        self.0
            .to_str()
            .map_err(|_| PathError::InvalidUnicode(self.0.as_bytes().to_str_lossy().to_string()))
    }

    pub fn make_canonical_for_tar(mut self, is_dir: bool) -> AnchoredUnixTarPathBuf {
        if is_dir && !self.0.ends_with(b"/") {
            self.0.push(b'/');
        }

        AnchoredUnixTarPathBuf(self.0)
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
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\"' || **byte == b'\n')
        {
            writer.write_all(&self.0[i..to_escaped_index])?;
            writer.write_all(&[b'\\', *byte])?;
            i = to_escaped_index + 1;
        }
        if i < self.0.len() {
            writer.write_all(&self.0[i..])?;
        }
        writer.write_all(&[b'\"'])?;
        Ok(())
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
        let buffer = Vec::with_capacity(self.0.len() + 1 + tail.0.len());
        let mut path = BString::new(buffer);
        if self.0.len() > 0 {
            path.extend_from_slice(&self.0);
            path.push(b'/');
        }
        path.extend_from_slice(&tail.0);
        Self(path)
    }
}

impl Borrow<RelativeUnixPath> for RelativeUnixPathBuf {
    fn borrow(&self) -> &RelativeUnixPath {
        let inner: &BStr = self.0.borrow();
        unsafe { &*(inner as *const BStr as *const RelativeUnixPath) }
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
        match self.as_str() {
            Ok(s) => write!(f, "{}", s),
            Err(s) => write!(f, "Non-utf8 {:?}", s),
        }
    }
}

impl TryFrom<&AnchoredSystemPath> for RelativeUnixPathBuf {
    type Error = PathError;

    fn try_from(path: &AnchoredSystemPath) -> Result<Self, Self::Error> {
        let path = path.to_str()?;
        Self::new(path)
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
        assert_eq!(path.as_str().unwrap(), "foo/bar");
    }

    #[test]
    fn test_relative_unix_path_buf_with_extension() {
        let path = RelativeUnixPathBuf::new("foo/bar.txt").unwrap();
        assert_eq!(path.as_str().unwrap(), "foo/bar.txt");
    }

    #[test]
    fn test_join() {
        let head = RelativeUnixPathBuf::new("some/path").unwrap();
        let tail = RelativeUnixPathBuf::new("child/leaf").unwrap();
        let combined = head.join(&tail);
        assert_eq!(combined.as_str().unwrap(), "some/path/child/leaf");
    }

    #[test]
    fn test_strip_prefix() {
        let combined = RelativeUnixPathBuf::new("some/path/child/leaf").unwrap();
        let head = RelativeUnixPathBuf::new("some/path").unwrap();
        let expected = RelativeUnixPathBuf::new("child/leaf").unwrap();
        let tail = combined.strip_prefix(head).unwrap();
        assert_eq!(tail, expected);
    }

    #[test]
    fn test_strip_entire_contents() {
        let combined = RelativeUnixPathBuf::new("some/path").unwrap();
        let head = combined.clone();
        let expected = RelativeUnixPathBuf::new("").unwrap();
        let tail = combined.strip_prefix(head).unwrap();
        assert_eq!(tail, expected);
    }

    #[test]
    fn test_strip_empty_prefix() {
        let combined = RelativeUnixPathBuf::new("some/path").unwrap();
        let tail = combined
            .strip_prefix(RelativeUnixPathBuf::new("").unwrap())
            .unwrap();
        assert_eq!(tail, combined);
    }

    #[test]
    fn test_write_escaped() {
        let input = "\"quote\"\nnewline\n".as_bytes();
        let expected = "\"\\\"quote\\\"\\\nnewline\\\n\"".as_bytes();
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
