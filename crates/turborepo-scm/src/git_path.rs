use std::{borrow::Cow, fmt::Write, path::Path};

use turbopath::RelativeUnixPathBuf;

use crate::Error;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct UnsupportedGitPath {
    source: &'static str,
    bytes: Vec<u8>,
}

impl UnsupportedGitPath {
    pub fn new(source: &'static str, bytes: &[u8]) -> Self {
        Self {
            source,
            bytes: bytes.to_vec(),
        }
    }

    pub fn is_within_prefix(&self, prefix: &RelativeUnixPathBuf) -> bool {
        let prefix = prefix.as_str().as_bytes();
        let path = self.bytes.as_slice();

        prefix.is_empty()
            || path == prefix
            || (path.len() > prefix.len() && path.starts_with(prefix) && path[prefix.len()] == b'/')
    }

    pub fn into_error(self) -> Error {
        Error::UnsupportedGitPath {
            origin: self.source,
            path: escape_bytes(&self.bytes),
            backtrace: std::backtrace::Backtrace::capture(),
        }
    }
}

pub(crate) fn parse_git_path(
    bytes: &[u8],
    source: &'static str,
) -> Result<Result<RelativeUnixPathBuf, UnsupportedGitPath>, Error> {
    let path = match std::str::from_utf8(bytes) {
        Ok(path) => path,
        Err(_) => return Ok(Err(UnsupportedGitPath::new(source, bytes))),
    };

    Ok(Ok(RelativeUnixPathBuf::new(path)?))
}

pub(crate) fn require_git_path(
    bytes: &[u8],
    source: &'static str,
) -> Result<RelativeUnixPathBuf, Error> {
    match parse_git_path(bytes, source)? {
        Ok(path) => Ok(path),
        Err(path) => Err(path.into_error()),
    }
}

pub(crate) fn parse_path(
    path: &Path,
    source: &'static str,
) -> Result<Result<RelativeUnixPathBuf, UnsupportedGitPath>, Error> {
    let bytes = path_to_git_path_bytes(path);
    parse_git_path(bytes.as_ref(), source)
}

pub(crate) fn path_to_git_path_bytes(path: &Path) -> Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        Cow::Borrowed(path.as_os_str().as_bytes())
    }

    #[cfg(windows)]
    {
        Cow::Owned(path.to_string_lossy().replace('\\', "/").into_bytes())
    }
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::with_capacity(bytes.len());
    for byte in bytes {
        match byte {
            b'\\' => escaped.push_str("\\\\"),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            b'\t' => escaped.push_str("\\t"),
            b'\0' => escaped.push_str("\\0"),
            0x20..=0x7e => escaped.push(*byte as char),
            _ => {
                let _ = write!(escaped, "\\x{byte:02x}");
            }
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use turbopath::RelativeUnixPathBuf;

    use super::{UnsupportedGitPath, parse_git_path};
    use crate::Error;

    #[test]
    fn parse_git_path_reports_non_utf8() {
        let parsed = parse_git_path(b"pkg/bad-\xff", "test source").unwrap();
        let path = parsed.unwrap_err();
        assert!(path.is_within_prefix(&RelativeUnixPathBuf::new("pkg").unwrap()));

        let err = path.into_error();
        match err {
            Error::UnsupportedGitPath { origin, path, .. } => {
                assert_eq!(origin, "test source");
                assert_eq!(path, "pkg/bad-\\xff");
            }
            _ => panic!("expected unsupported git path error"),
        }
    }

    #[test]
    fn unsupported_path_prefix_matching_respects_components() {
        let path = UnsupportedGitPath::new("test source", b"packages/app/file-\xff");

        assert!(path.is_within_prefix(&RelativeUnixPathBuf::new("").unwrap()));
        assert!(path.is_within_prefix(&RelativeUnixPathBuf::new("packages/app").unwrap()));
        assert!(!path.is_within_prefix(&RelativeUnixPathBuf::new("packages/ap").unwrap()));
        assert!(!path.is_within_prefix(&RelativeUnixPathBuf::new("apps/web").unwrap()));
    }
}
