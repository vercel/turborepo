#![cfg(feature = "git2")]

use tracing::{debug, trace};
use turbopath::RelativeUnixPathBuf;

use crate::{Error, GitHashes, GitRepo, ls_tree::SortedGitHashes, status::RepoStatusEntry};

/// Pre-computed repo-wide git index that caches the results of `git ls-tree`
/// and `git status` so they can be filtered per-package without spawning
/// additional subprocesses.
///
/// Uses a `BTreeMap` for the ls-tree data so that per-package lookups can
/// use `range()` on the sorted keys instead of scanning every entry.
pub struct RepoGitIndex {
    ls_tree_hashes: SortedGitHashes,
    status_entries: Vec<RepoStatusEntry>,
}

impl RepoGitIndex {
    #[tracing::instrument(skip(git))]
    pub fn new(git: &GitRepo) -> Result<Self, Error> {
        // These two git commands are independent: ls-tree reads the committed
        // tree while status reads the working directory. Run them on separate
        // threads so the wall-clock cost is max(ls_tree, status) instead of
        // their sum.
        let (ls_tree_hashes, status_entries) = std::thread::scope(|s| {
            let ls_tree = s.spawn(|| git.git_ls_tree_repo_root_sorted());
            let status = s.spawn(|| git.git_status_repo_root());
            (
                ls_tree.join().expect("ls-tree thread panicked"),
                status.join().expect("status thread panicked"),
            )
        });
        let ls_tree_hashes = ls_tree_hashes?;
        let status_entries = status_entries?;

        debug!(
            "built repo git index: ls_tree_count={}, status_count={}",
            ls_tree_hashes.len(),
            status_entries.len(),
        );
        Ok(Self {
            ls_tree_hashes,
            status_entries,
        })
    }

    /// Extract hashes for a single package from the cached repo-wide data.
    ///
    /// Returns `(hashes, to_hash)` where:
    /// - `hashes` contains committed file hashes keyed by package-relative
    ///   paths
    /// - `to_hash` contains git-root-relative paths for files that need hashing
    ///   (modified/untracked files within the package)
    pub fn get_package_hashes(
        &self,
        pkg_prefix: &RelativeUnixPathBuf,
    ) -> Result<(GitHashes, Vec<RelativeUnixPathBuf>), Error> {
        let prefix_str = pkg_prefix.as_str();
        let prefix_is_empty = prefix_str.is_empty();

        let mut hashes = if prefix_is_empty {
            // Full-repo case: pre-allocate to the exact ls-tree count.
            let mut h = GitHashes::with_capacity(self.ls_tree_hashes.len());
            for (path, hash) in &self.ls_tree_hashes {
                h.insert(path.clone(), hash.clone());
            }
            h
        } else {
            let range_start = RelativeUnixPathBuf::new(format!("{}/", prefix_str)).unwrap();
            let range_end = RelativeUnixPathBuf::new(format!("{}0", prefix_str)).unwrap();
            let range = self.ls_tree_hashes.range(range_start..range_end);
            // Use size_hint to pre-allocate; BTreeMap ranges provide a lower
            // bound based on the tree structure.
            let (lower, _) = range.size_hint();
            let mut h = GitHashes::with_capacity(lower);
            for (path, hash) in range {
                if let Ok(stripped) = path.strip_prefix(pkg_prefix) {
                    h.insert(stripped, hash.clone());
                }
            }
            h
        };

        let mut to_hash = Vec::new();
        for entry in &self.status_entries {
            let path_str = entry.path.as_str();
            let belongs_to_package = if prefix_is_empty {
                true
            } else {
                path_str.starts_with(prefix_str)
                    && path_str.as_bytes().get(prefix_str.len()) == Some(&b'/')
            };

            if !belongs_to_package {
                continue;
            }

            if entry.is_delete {
                if let Ok(stripped) = entry.path.strip_prefix(pkg_prefix) {
                    hashes.remove(&stripped);
                }
            } else {
                to_hash.push(entry.path.clone());
            }
        }

        trace!(
            "filtered repo index for package: pkg_prefix={:?}, ls_tree_matched={}, \
             to_hash_count={}",
            prefix_str,
            hashes.len(),
            to_hash.len(),
        );

        Ok((hashes, to_hash))
    }
}
