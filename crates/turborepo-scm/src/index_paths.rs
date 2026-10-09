//! The paths a git index records, read without the `git` binary.
//!
//! Callers that must never touch tracked content (e.g. `turbo clean`) need a
//! complete answer or none at all. Unlike the best-effort repository index
//! used for hashing, this never falls back: it works without a `git` binary
//! on `PATH`, and a repository whose index is missing or unreadable is an
//! error rather than "nothing is tracked".

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{Error, worktree::resolve_git_dir};

/// Every path recorded in the git index of a repository.
#[derive(Debug, Clone)]
pub struct IndexPaths {
    /// The real path of the working tree root the paths are relative to.
    pub git_root: AbsoluteSystemPathBuf,
    /// Tracked files and symlinks: `/`-separated, relative to `git_root`,
    /// sorted.
    pub files: Vec<String>,
    /// Directory entries: submodules (gitlinks) and the directories of a
    /// sparse index. Everything below them belongs to another tree.
    pub directories: Vec<String>,
}

impl IndexPaths {
    /// Reads the index of the git repository containing `path_in_repo`,
    /// found by looking for a `.git` entry in it or one of its ancestors.
    ///
    /// Returns `Ok(None)` when no ancestor has a `.git` entry. Errors when a
    /// repository is found but its index is missing or cannot be read.
    pub fn read(path_in_repo: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let real_path = path_in_repo.to_realpath()?;
        let Some(git_root) = real_path
            .as_std_path()
            .ancestors()
            .find(|dir| std::fs::symlink_metadata(dir.join(".git")).is_ok())
        else {
            return Ok(None);
        };
        let git_root = AbsoluteSystemPathBuf::try_from(git_root)?;
        let git_dir = resolve_git_dir(&git_root)?;
        let index_path = git_dir.join_component("index");
        if !index_path.exists() {
            return Err(Error::git_error(format!(
                "the git repository at {git_root} has no index file"
            )));
        }
        let index = gix_index::File::at(
            index_path.as_std_path(),
            gix_index::hash::Kind::Sha1,
            false,
            gix_index::decode::Options::default(),
        )
        .map_err(|e| Error::git_error(format!("failed to read git index: {e}")))?;

        let mut files = Vec::with_capacity(index.entries().len());
        let mut directories = Vec::new();
        for entry in index.entries() {
            let path = String::from_utf8_lossy(entry.path(&index)).into_owned();
            if entry.mode.is_submodule() || entry.mode.is_sparse() {
                directories.push(path);
            } else {
                files.push(path);
            }
        }
        files.sort();
        files.dedup();
        directories.sort();
        directories.dedup();
        Ok(Some(Self {
            git_root,
            files,
            directories,
        }))
    }

    /// Whether `path` (relative to `git_root`, `/`-separated) is a tracked
    /// file or symlink, compared byte for byte.
    pub fn contains_file(&self, path: &str) -> bool {
        self.files
            .binary_search_by(|entry| entry.as_str().cmp(path))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn git(root: &AbsoluteSystemPath, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    fn repo() -> (TempDir, AbsoluteSystemPathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp, root)
    }

    #[test]
    fn outside_git_is_none() {
        let (_tmp, root) = repo();
        assert!(IndexPaths::read(&root).unwrap().is_none());
    }

    #[test]
    fn a_repository_without_an_index_is_an_error() {
        let (_tmp, root) = repo();
        git(&root, &["init", "--quiet"]);
        assert!(IndexPaths::read(&root).is_err());
    }

    #[test]
    fn records_files_symlinks_and_gitlinks() {
        let (_tmp, root) = repo();
        git(&root, &["init", "--quiet"]);
        let nested = root.join_components(&["pkg", "src"]);
        nested.create_dir_all().unwrap();
        nested
            .join_component("index.ts")
            .create_with_contents("source")
            .unwrap();
        #[cfg(unix)]
        root.join_components(&["pkg", "link.js"])
            .symlink_to_file("src/index.ts")
            .unwrap();
        git(&root, &["add", "."]);
        // A submodule entry, added without cloning anything.
        git(
            &root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                "160000,5b5dbe4fa71b9ba5e09e1e0b5d2bd8ea2b0d4a4d,pkg/vendor",
            ],
        );

        // Read from a nested directory: the repository is found above it.
        let paths = IndexPaths::read(&nested).unwrap().unwrap();
        assert_eq!(paths.git_root, root);
        assert!(paths.contains_file("pkg/src/index.ts"));
        #[cfg(unix)]
        assert!(paths.contains_file("pkg/link.js"));
        assert!(!paths.contains_file("pkg/vendor"));
        assert_eq!(paths.directories, ["pkg/vendor"]);
    }
}
