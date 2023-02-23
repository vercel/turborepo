use std::path::Path;

/// A path that is anchored, unix style, and always ends in '/'
/// when pointing to a directory
pub struct AnchoredUnixTarPathBuf(pub(crate) String);

impl AnchoredUnixTarPathBuf {
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}
