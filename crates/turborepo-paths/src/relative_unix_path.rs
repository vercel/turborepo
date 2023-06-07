use std::path::PathBuf;

use bstr::BStr;

use crate::{PathError, RelativeUnixPathBuf};

#[repr(transparent)]
pub struct RelativeUnixPath(pub(crate) BStr);

impl RelativeUnixPath {
    pub fn new<P: AsRef<BStr>>(value: &P) -> Result<&Self, PathError> {
        let path = value.as_ref();
        if path.first() == Some(&b'/') {
            return Err(PathError::not_relative_error(path));
        }
        // copied from stdlib path.rs: relies on the representation of
        // RelativeUnixPath being just a BStr, the same way Path relies on
        // just being an OsStr
        Ok(unsafe { &*(path as *const BStr as *const Self) })
    }

    pub(crate) fn to_system_path_buf(&self) -> Result<PathBuf, PathError> {
        #[cfg(unix)]
        {
            // On unix, unix paths are already system paths. Copy the bytes
            // but skip validation.
            use std::{ffi::OsString, os::unix::prelude::OsStringExt};
            Ok(PathBuf::from(OsString::from_vec(self.0.to_vec())))
        }

        #[cfg(windows)]
        {
            let system_path_bytes = self
                .0
                .iter()
                .map(|byte| if *byte == b'/' { b'\\' } else { *byte })
                .collect::<Vec<u8>>();
            let system_path_string = String::from_utf8(system_path_bytes).map_err(|err| {
                PathError::InvalidUnicode(String::from_utf8_lossy(err.as_bytes()).to_string())
            })?;
            Ok(PathBuf::from(system_path_string))
        }
    }

    pub fn to_owned(&self) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf(self.0.to_owned())
    }

    pub fn strip_prefix(
        &self,
        prefix: impl AsRef<RelativeUnixPath>,
    ) -> Result<RelativeUnixPathBuf, PathError> {
        let prefix = prefix.as_ref();
        let prefix_len = prefix.0.len();
        if prefix_len == 0 {
            return Ok(RelativeUnixPathBuf(self.0.to_owned()));
        }
        if !self.0.starts_with(&prefix.0) {
            return Err(PathError::NotParent(
                prefix.0.to_string(),
                self.0.to_string(),
            ));
        }

        // Handle the case where we are stripping the entire contents of this path
        if self.0.len() == prefix.0.len() {
            return RelativeUnixPathBuf::new("");
        }

        // We now know that this path starts with the prefix, and that this path's
        // length is greater than the prefix's length
        if self.0[prefix_len] != b'/' {
            let prefix_str = prefix.0.to_string();
            let this = self.0.to_string();
            return Err(PathError::PrefixError(prefix_str, this));
        }

        let tail_slice = &self.0[(prefix_len + 1)..];
        RelativeUnixPathBuf::new(tail_slice.to_vec())
    }

    pub fn ends_with(&self, suffix: impl AsRef<[u8]>) -> bool {
        self.0.ends_with(suffix.as_ref())
    }
}

impl AsRef<RelativeUnixPath> for RelativeUnixPath {
    fn as_ref(&self) -> &RelativeUnixPath {
        self
    }
}
