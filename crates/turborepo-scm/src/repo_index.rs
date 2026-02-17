#![cfg(feature = "git2")]

use std::{
    collections::BTreeMap,
    sync::{Condvar, Mutex},
};

use tracing::{debug, trace};
use turbopath::RelativeUnixPathBuf;

use crate::{Error, GitHashes, GitRepo, status::RepoStatusEntry};

/// Limits concurrent file-system operations to avoid exhausting file
/// descriptors when many rayon threads are hashing simultaneously.
pub(crate) struct IoSemaphore {
    state: Mutex<usize>,
    cond: Condvar,
    max: usize,
}

impl IoSemaphore {
    fn new(max: usize) -> Self {
        Self {
            state: Mutex::new(0),
            cond: Condvar::new(),
            max,
        }
    }

    pub(crate) fn acquire(&self) -> IoPermit<'_> {
        let mut count = self.state.lock().unwrap();
        while *count >= self.max {
            count = self.cond.wait(count).unwrap();
        }
        *count += 1;
        IoPermit(self)
    }
}

pub(crate) struct IoPermit<'a>(&'a IoSemaphore);

impl Drop for IoPermit<'_> {
    fn drop(&mut self) {
        let mut count = self.0.state.lock().unwrap();
        *count -= 1;
        self.0.cond.notify_one();
    }
}

/// Pre-computed repo-wide git index that caches the results of `git ls-tree`
/// and `git status` so they can be filtered per-package without spawning
/// additional subprocesses.
///
/// Uses a `BTreeMap` for the ls-tree data so that per-package lookups can
/// use `range()` on the sorted keys instead of scanning every entry.
pub struct RepoGitIndex {
    ls_tree_hashes: BTreeMap<RelativeUnixPathBuf, String>,
    status_entries: Vec<RepoStatusEntry>,
    pub(crate) io_semaphore: IoSemaphore,
}

impl RepoGitIndex {
    #[tracing::instrument(skip(git))]
    pub fn new(git: &GitRepo) -> Result<Self, Error> {
        // These two git commands are independent: ls-tree reads the committed
        // tree while status reads the working directory. Run them on separate
        // threads so the wall-clock cost is max(ls_tree, status) instead of
        // their sum.
        let (raw_hashes, status_entries) = std::thread::scope(|s| {
            let ls_tree = s.spawn(|| git.git_ls_tree_repo_root());
            let status = s.spawn(|| git.git_status_repo_root());
            (
                ls_tree.join().expect("ls-tree thread panicked"),
                status.join().expect("status thread panicked"),
            )
        });
        let raw_hashes = raw_hashes?;
        let status_entries = status_entries?;

        // Convert HashMap to BTreeMap for sorted prefix-range lookups
        let ls_tree_hashes: BTreeMap<RelativeUnixPathBuf, String> =
            raw_hashes.into_iter().collect();

        debug!(
            "built repo git index: ls_tree_count={}, status_count={}",
            ls_tree_hashes.len(),
            status_entries.len(),
        );
        Ok(Self {
            ls_tree_hashes,
            status_entries,
            io_semaphore: IoSemaphore::new(MAX_CONCURRENT_GLOBWALKS),
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

        // Use BTreeMap range to only iterate files under this package prefix,
        // rather than scanning the entire repo's file list.
        let mut hashes = GitHashes::new();
        if prefix_is_empty {
            // Root package — all files belong
            for (path, hash) in &self.ls_tree_hashes {
                hashes.insert(path.clone(), hash.clone());
            }
        } else {
            // Compute the range [prefix/, prefix0) where '0' is '/' + 1.
            // This captures all paths that start with "prefix/".
            let range_start = RelativeUnixPathBuf::new(format!("{}/", prefix_str)).unwrap();
            let range_end = RelativeUnixPathBuf::new(format!("{}0", prefix_str)).unwrap();
            for (path, hash) in self.ls_tree_hashes.range(range_start..range_end) {
                if let Ok(stripped) = path.strip_prefix(pkg_prefix) {
                    hashes.insert(stripped, hash.clone());
                }
            }
        }

        // Status entries are typically a small list (only modified/untracked
        // files), so a linear scan is fine.
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

/// Maximum number of concurrent globwalk operations.
///
/// globwalk internally uses rayon parallel iteration (into_par_iter)
/// and walkdir which holds directory handles open during traversal.
/// A single globwalk in a large monorepo can open hundreds of directory
/// handles as it traverses deep node_modules trees. Multiple concurrent
/// globwalks compound this to thousands of open fds.
///
/// Before the repo index optimization, per-package git subprocesses
/// blocked rayon threads for hundreds of milliseconds, naturally
/// preventing more than 1-2 globwalks from running simultaneously.
/// With the index, threads reach globwalk near-instantly and the
/// parallel explosion happens.
///
/// We serialize globwalk operations (limit=1) to match the old
/// effective behavior. The common path (packages without custom inputs)
/// uses the index-only path which runs fully parallel without this
/// semaphore.
const MAX_CONCURRENT_GLOBWALKS: usize = 1;
