//! The user's tool store: one private directory per user, outside every
//! repository, that holds the shared trees and environments plus setup's
//! per-repository state. `TURBO_TOOL_STORE` relocates it, for example to a
//! directory CI caches.

use std::{
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{Error, real_directory};

/// Overrides the default store location.
pub const TOOL_STORE_VAR: &str = "TURBO_TOOL_STORE";

/// A located store root: absolute, canonical, a real directory owned by this
/// user that nobody else can write, and neither inside nor around the
/// repository it was located for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserStore {
    pub(crate) root: PathBuf,
}

impl UserStore {
    /// `$TURBO_TOOL_STORE`, else `<data_local_dir>/turborepo/tools`, checked
    /// and created like [`UserStore::at`]. An empty variable counts as unset.
    pub fn locate(repo: &Path) -> Result<Self, Error> {
        Self::at(&Self::default_root()?, repo)
    }

    /// Where [`UserStore::locate`] looks, unchecked and never created.
    pub fn default_root() -> Result<PathBuf, Error> {
        Ok(
            match std::env::var_os(TOOL_STORE_VAR).filter(|path| !path.is_empty()) {
                Some(path) => PathBuf::from(path),
                None => dirs_next::data_local_dir()
                    .ok_or(Error::NoStore)?
                    .join("turborepo")
                    .join("tools"),
            },
        )
    }

    /// The store at `path`, creating missing directories with mode 0700.
    /// Rejects a relative path, `..` components, a symlinked or foreign store,
    /// and a store inside `repo` or containing it.
    pub fn at(path: &Path, repo: &Path) -> Result<Self, Error> {
        if !cfg!(any(unix, windows)) {
            return Err(Error::UnsupportedPlatform);
        }
        let unsafe_store = || Error::UnsafeStore(path.to_owned());
        if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
            return Err(unsafe_store());
        }
        let repo = fs::canonicalize(repo)?;
        // Resolve the existing ancestor, so the containment check runs before
        // anything is created. A dangling link is never a new component.
        let mut existing = path;
        let mut missing: Vec<OsString> = Vec::new();
        while fs::symlink_metadata(existing).is_err() {
            missing.push(existing.file_name().ok_or_else(unsafe_store)?.to_owned());
            existing = existing.parent().ok_or_else(unsafe_store)?;
        }
        let base = fs::canonicalize(existing)?;
        let mut root = base.clone();
        root.extend(missing.iter().rev());
        if root.starts_with(&repo) || repo.starts_with(&root) {
            return Err(unsafe_store());
        }
        let mut created = base;
        for part in missing.iter().rev() {
            created.push(part);
            create_private(&created)?;
        }
        // Ancestors may be links, such as macOS's `/var`; the store may not.
        match real_directory(path) {
            Err(Error::UnsafePath) => return Err(unsafe_store()),
            result => result?,
        }
        if fs::canonicalize(path)? != root {
            return Err(unsafe_store());
        }
        // Environments embed absolute store paths in `.cmd` forwarders, which
        // cmd.exe cannot run under a verbatim `\\?\` prefix.
        #[cfg(windows)]
        let root = dunce::simplified(&root).to_path_buf();
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory holding every repository's state.
    pub fn repos(&self) -> PathBuf {
        self.root.join("repos")
    }

    /// One repository's state directory, named by the SHA-256 of its
    /// canonical root's native bytes, so non-UTF-8 roots never collide.
    pub fn repo(&self, canonical_root: &Path) -> PathBuf {
        Self::repo_in(&self.root, canonical_root)
    }

    /// [`UserStore::repo`] under an unchecked store root, for lookups that
    /// must not create or validate the store.
    pub fn repo_in(store: &Path, canonical_root: &Path) -> PathBuf {
        let name = hex::encode(Sha256::digest(
            canonical_root.as_os_str().as_encoded_bytes(),
        ));
        store.join("repos").join(name)
    }

    /// The repository's private `CARGO_HOME`, which Cargo creates on first
    /// use.
    pub fn cargo_home(&self, canonical_root: &Path) -> PathBuf {
        self.repo(canonical_root).join("cargo-home")
    }
}

fn create_private(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)
    };
    #[cfg(not(unix))]
    let created = fs::create_dir(path);
    match created {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => Err(e.into()),
        _ => Ok(()),
    }
}
