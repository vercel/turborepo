use std::path::PathBuf;

use bstr::BStr;

use crate::{PathError, PathValidationError};

#[repr(transparent)]
pub struct RelativeUnixPath {
    inner: BStr,
}

impl RelativeUnixPath {
    pub fn new<P: AsRef<BStr>>(value: &P) -> Result<&Self, PathError> {
        let path = value.as_ref();
        if path.first() == Some(&b'/') {
            return Err(PathValidationError::not_relative_error(path).into());
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
            Ok(PathBuf::from(OsString::from_vec(self.inner.to_vec())))
        }

        #[cfg(windows)]
        {
            let system_path_bytes = self
                .inner
                .iter()
                .map(|byte| if *byte == b'/' { b'\\' } else { *byte })
                .collect::<Vec<u8>>();
            let system_path_string = String::from_utf8(system_path_bytes)?;
            Ok(PathBuf::from(system_path_string))
        }
    }
}
