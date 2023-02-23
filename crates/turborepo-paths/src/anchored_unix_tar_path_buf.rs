use std::path::Path;

use bstr::{BString, ByteSlice};

use crate::PathError;

/// A path that is anchored, unix style, and always ends in '/'
/// when pointing to a directory
pub struct AnchoredUnixTarPathBuf(pub(crate) BString);

impl AnchoredUnixTarPathBuf {
    pub fn as_path(&self) -> Result<&Path, PathError> {
        Ok(self.0.to_path()?)
    }
}
