use std::path::PathBuf;

use bstr::BStr;

use crate::{not_relative_error, PathError, RelativeSystemPathBuf};

#[repr(transparent)]
pub struct RelativeUnixPath {
    inner: BStr,
}

impl RelativeUnixPath {
    pub fn new<P: AsRef<BStr>>(value: &P) -> Result<&Self, PathError> {
        let path = value.as_ref();
        if path[0] == b'/' {
            return Err(not_relative_error(path).into());
        }
        // copied from stdlib path.rs: relies on the representation of
        // RelativeUnixPath being just a Path, the same way Path relies on
        // just being an OsStr
        Ok(unsafe { &*(path as *const BStr as *const Self) })
    }

    pub fn to_system_path(&self) -> Result<RelativeSystemPathBuf, PathError> {
        #[cfg(unix)]
        {
            // On unix, unix paths are already system paths. Copy the bytes
            // but skip validation.
            use std::{ffi::OsString, os::unix::prelude::OsStringExt};
            let path = PathBuf::from(OsString::from_vec(self.inner.to_vec()));
            Ok(RelativeSystemPathBuf::new_unchecked(path))
        }

        #[cfg(windows)]
        {
            let system_path_bytes = self
                .inner
                .iter()
                .map(|byte| if *byte == b'/' { b'\\' } else { *byte })
                .collect::<Vec<u8>>();
            // Is this safe to do? We think we have utf8 bytes or bytes that roundtrip
            // through utf8
            let system_path_string = unsafe { String::from_utf8_unchecked(system_path_bytes) };
            let system_path_buf = PathBuf::from(system_path_string);
            Ok(RelativeSystemPathBuf::new_unchecked(system_path_buf))
        }
    }
}
