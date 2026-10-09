//! Filesystem identity: which file a name refers to, as the filesystem
//! itself decides it, including its case and Unicode equivalences.

use std::{collections::HashMap, path::PathBuf};

/// A file's device and inode. Two names with the same identity are the same
/// file, whatever their spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) struct FileId {
    pub(crate) dev: u64,
    pub(crate) ino: u64,
}

impl FileId {
    #[cfg(unix)]
    fn of(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }
}

/// The identity of `path` itself, never following a final symlink. `None`
/// when it does not exist, or where identities are not available.
pub(crate) fn lstat(path: &std::path::Path) -> Option<FileId> {
    #[cfg(unix)]
    {
        std::fs::symlink_metadata(path)
            .ok()
            .map(|metadata| FileId::of(&metadata))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// The identity of what `path` resolves to, following symlinks.
pub(crate) fn stat(path: &std::path::Path) -> Option<FileId> {
    #[cfg(unix)]
    {
        std::fs::metadata(path)
            .ok()
            .map(|metadata| FileId::of(&metadata))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Identities of paths under the real repository root, looked up once each.
pub(crate) struct Identities {
    root: PathBuf,
    cache: HashMap<Vec<String>, Option<FileId>>,
}

impl Identities {
    pub(crate) fn new(real_root: &std::path::Path) -> Self {
        Self {
            root: real_root.to_owned(),
            cache: HashMap::new(),
        }
    }

    pub(crate) fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The identity of the stored path `components` below the root.
    pub(crate) fn of(&mut self, components: &[String]) -> Option<FileId> {
        if let Some(id) = self.cache.get(components) {
            return *id;
        }
        let mut path = self.root.clone();
        path.extend(components);
        let id = lstat(&path);
        self.cache.insert(components.to_vec(), id);
        id
    }
}
