use tracing::{debug, trace};
use turbopath::RelativeUnixPathBuf;

use crate::{
    Error, GitHashes, GitRepo, OidHash,
    git_path::{UnsupportedGitPath, parse_git_path, parse_path, path_to_git_path_bytes},
    ls_tree::{GitPathList, SortedGitHashes},
    status::RepoStatusEntry,
};

/// Pre-computed repo-wide git index that caches file hashes and working-tree
/// status so they can be filtered per-package without spawning additional
/// subprocesses.
///
/// Both collections are sorted by path so that per-package lookups can use
/// `partition_point` (binary search) for range queries. This gives O(log n)
/// lookup cost with good cache locality on contiguous memory.
pub struct RepoGitIndex {
    ls_tree_hashes: SortedGitHashes,
    /// Sorted by path so per-package filtering can use binary-search range
    /// queries instead of linear scans.
    status_entries: Vec<RepoStatusEntry>,
    /// Untracked symlinks discovered by the working-tree walk. Kept out of
    /// `status_entries` so per-package hashing never attempts to hash them,
    /// while dirty-hash provenance still reflects them (git tracks symlinks).
    /// Only populated by [`Self::populate_untracked_for_prefixes`] /
    /// [`Self::populate_all_untracked`].
    untracked_symlinks: Vec<RelativeUnixPathBuf>,
    unsupported_paths: Vec<UnsupportedGitPath>,
    untracked_entries_populated: bool,
}

impl RepoGitIndex {
    #[cfg(test)]
    pub(crate) fn new_for_testing(
        ls_tree_hashes: SortedGitHashes,
        status_entries: Vec<RepoStatusEntry>,
    ) -> Self {
        Self {
            ls_tree_hashes,
            status_entries,
            unsupported_paths: Vec::new(),
            untracked_symlinks: Vec::new(),
            untracked_entries_populated: true,
        }
    }

    #[tracing::instrument(skip(git))]
    pub fn new(git: &GitRepo) -> Result<Self, Error> {
        let mut index = Self::new_tracked(git)?;
        index.populate_all_untracked(git)?;
        Ok(index)
    }

    #[tracing::instrument(skip(git))]
    pub fn new_tracked(git: &GitRepo) -> Result<Self, Error> {
        Self::new_from_gix_index(git)
    }

    /// Build the index by reading `.git/index` directly via gix-index.
    ///
    /// This replaces both `git ls-tree` and `git status` with a single
    /// operation: reading the index file gives us committed blob OIDs, and
    /// stat-comparing each entry against the filesystem tells us which files
    /// are modified or deleted. Untracked files can be layered on later once
    /// the caller knows which package prefixes actually need them.
    ///
    /// Racy-git entries (where mtime >= index timestamp, so we can't trust
    /// the stat comparison) are deferred to per-package hashing rather than
    /// content-hashed inline. This avoids reading every file from disk on
    /// freshly cloned/checked-out repos.
    #[tracing::instrument(skip(git))]
    fn new_from_gix_index(git: &GitRepo) -> Result<Self, Error> {
        use rayon::prelude::*;

        let git_dir = git.root.join_component(".git");
        let index_path = git_dir.join_component("index");

        if !index_path.exists() {
            return Err(Error::git_error("no .git/index file found"));
        }

        let index = gix_index::File::at(
            index_path.as_std_path(),
            gix_index::hash::Kind::Sha1,
            true, // skip_hash: don't verify the index checksum (2x faster)
            gix_index::decode::Options::default(),
        )
        .map_err(|e| Error::git_error(format!("failed to read git index: {}", e)))?;

        let stat_opts = gix_index::entry::stat::Options {
            trust_ctime: true,
            check_stat: true,
            // Nanosecond precision reduces false racy-git entries on modern
            // filesystems (macOS APFS, Linux ext4/btrfs).
            use_nsec: true,
            use_stdev: false,
        };

        let index_timestamp = index.timestamp();

        // The index is sorted by path. rayon's indexed collect preserves
        // order, and our sequential collection loop preserves order, so
        // ls_tree_hashes will be sorted without an explicit sort.
        // Use the total entry count as a capacity hint (slightly over-
        // estimates when submodules are present, but avoids a full
        // sequential scan of all entries).
        let num_entries = index.entries().len();

        // Classify entries in parallel: stat each file, compare with index,
        // and carry the raw ObjectId (20 bytes, Copy) instead of a heap-allocated
        // hex String. Hex conversion uses a thread-local stack buffer to avoid
        // allocator contention across rayon threads.
        let classified: Vec<Result<EntryClassification, Error>> = index
            .entries()
            .par_iter()
            .filter(|e| !e.mode.is_submodule())
            .map(|e| {
                let path_bytes = e.path(&index);
                let rel_path = match parse_git_path(path_bytes, "git index path")? {
                    Ok(path) => path,
                    Err(path) => return Ok(EntryClassification::Unsupported(path)),
                };
                // Git index paths are normalized repo-relative paths, so avoid
                // per-entry path_clean normalization before stat calls.
                let abs_path = git.root.join_unix_path_unchecked(&rel_path);

                match gix_index::fs::Metadata::from_path_no_follow(abs_path.as_std_path()) {
                    Ok(fs_meta) => {
                        let fs_stat = gix_index::entry::Stat::from_fs(&fs_meta).map_err(|err| {
                            Error::git_error(format!(
                                "failed to convert stat for {}: {}",
                                rel_path, err
                            ))
                        })?;

                        let stat_matches = e.stat.matches(&fs_stat, stat_opts);

                        if !stat_matches {
                            return Ok(EntryClassification::Modified { path: rel_path });
                        }

                        let is_racy = e.stat.is_racy(index_timestamp, stat_opts);
                        if is_racy {
                            return Ok(EntryClassification::Modified { path: rel_path });
                        }

                        let mut hex_buf = [0u8; 40];
                        hex::encode_to_slice(e.id.as_bytes(), &mut hex_buf).map_err(|err| {
                            Error::git_error(format!(
                                "failed to encode object id for {rel_path}: {err}"
                            ))
                        })?;
                        Ok(EntryClassification::Clean {
                            path: rel_path,
                            oid: OidHash::from_hex_buf(hex_buf),
                        })
                    }
                    Err(_) => Ok(EntryClassification::Deleted { path: rel_path }),
                }
            })
            .collect();

        let mut ls_tree_hashes = SortedGitHashes::with_capacity(num_entries);
        let mut status_entries = Vec::new();
        let mut unsupported_paths = Vec::new();

        for result in classified {
            match result? {
                EntryClassification::Clean { path, oid } => {
                    ls_tree_hashes.push((path, oid));
                }
                EntryClassification::Modified { path } => {
                    status_entries.push(RepoStatusEntry {
                        path,
                        is_delete: false,
                        is_untracked: false,
                    });
                }
                EntryClassification::Deleted { path } => {
                    status_entries.push(RepoStatusEntry {
                        path,
                        is_delete: true,
                        is_untracked: false,
                    });
                }
                EntryClassification::Unsupported(path) => unsupported_paths.push(path),
            }
        }

        // ls_tree_hashes is already sorted (git index is sorted, rayon
        // preserves order for indexed iterators, sequential loop preserves
        // order). status_entries from Modified/Deleted are also in index order
        // (sorted). Sort once now so find_untracked_files can binary search
        // directly on &[RepoStatusEntry] without cloning paths into Strings.
        status_entries.sort_by(|a, b| a.path.cmp(&b.path));
        unsupported_paths.sort();
        unsupported_paths.dedup();

        debug!(
            "built tracked repo git index (gix-index): clean_count={}, status_count={}",
            ls_tree_hashes.len(),
            status_entries.len(),
        );

        Ok(Self {
            ls_tree_hashes,
            status_entries,
            unsupported_paths,
            untracked_symlinks: Vec::new(),
            untracked_entries_populated: false,
        })
    }

    /// Build the index from parallel git subprocesses + a race for untracked
    /// file discovery.
    ///
    /// Runs four operations concurrently:
    /// - `git ls-tree -r HEAD` for committed blob OIDs
    /// - `git diff-index HEAD` for modified/deleted files
    /// - `walk_candidate_files` (8-thread ignore-crate walk)
    /// - `git ls-files --others` (single-threaded git subprocess)
    ///
    /// The two untracked approaches race. On macOS, the multi-threaded walk
    /// typically wins (~440ms vs ~530ms). On Linux, the git subprocess wins
    /// (~230ms vs ~470ms). Using whichever finishes first guarantees no
    /// regressions on any platform.
    #[tracing::instrument(skip(git, prefixes))]
    pub fn new_from_subprocess_and_walk(
        git: &GitRepo,
        prefixes: &[RelativeUnixPathBuf],
    ) -> Result<Self, Error> {
        use std::{sync::mpsc, thread};

        enum UntrackedResult {
            /// Direct untracked file list from `git ls-files --others`
            LsFiles(Result<GitPathList, Error>),
            /// All candidate files (tracked + untracked) from the walk;
            /// must be filtered against ls_tree to find untracked
            Walk(Result<WalkedPaths, Error>),
        }

        let git1 = git.clone();
        let git2 = git.clone();
        let git3 = git.clone();
        let walk_root = git.root.as_std_path().to_path_buf();
        let walk_prefixes: Vec<_> = prefixes.to_vec();

        let ls_tree_handle =
            thread::spawn(move || git1.git_ls_tree_repo_root_sorted_with_unsupported());
        let diff_index_handle =
            thread::spawn(move || git2.git_diff_index_repo_root_with_unsupported());

        // Race: spawn both untracked discovery methods, use whichever
        // finishes first.
        let (untracked_tx, untracked_rx) = mpsc::channel();

        let tx1 = untracked_tx.clone();
        let _ls_files_handle = thread::spawn(move || {
            let result = git3.git_ls_files_untracked_with_unsupported();
            let _ = tx1.send(UntrackedResult::LsFiles(result));
        });

        let tx2 = untracked_tx;
        let _walk_handle = thread::spawn(move || {
            let result = walk_candidate_files_with_unsupported(&walk_root, Some(&walk_prefixes));
            let _ = tx2.send(UntrackedResult::Walk(result));
        });

        let tree_state = ls_tree_handle
            .join()
            .map_err(|_| Error::git_error("git ls-tree thread panicked"))??;
        let status_state = diff_index_handle
            .join()
            .map_err(|_| Error::git_error("git diff-index thread panicked"))??;
        let ls_tree_hashes = tree_state.hashes;
        let mut status_entries = status_state.entries;
        let mut unsupported_paths = tree_state.unsupported_paths;
        unsupported_paths.extend(status_state.unsupported_paths);

        // Use whichever untracked result arrives first.
        let untracked_winner = untracked_rx
            .recv()
            .map_err(|_| Error::git_error("both untracked discovery threads failed"))?;

        match untracked_winner {
            UntrackedResult::LsFiles(result) => {
                let untracked_files = result?;
                debug!(
                    "untracked race winner: git ls-files ({} files)",
                    untracked_files.paths.len()
                );
                unsupported_paths.extend(untracked_files.unsupported_paths);
                for path in untracked_files.paths {
                    status_entries.push(RepoStatusEntry {
                        path,
                        is_delete: false,
                        is_untracked: true,
                    });
                }
            }
            UntrackedResult::Walk(result) => {
                let candidates = result?;
                debug!(
                    "untracked race winner: walk ({} candidates)",
                    candidates.paths.len()
                );
                unsupported_paths.extend(candidates.unsupported_paths);
                let untracked = filter_untracked_from_candidates(
                    candidates.paths,
                    &ls_tree_hashes,
                    &status_entries,
                );
                for path in untracked {
                    status_entries.push(RepoStatusEntry {
                        path,
                        is_delete: false,
                        is_untracked: true,
                    });
                }
            }
        }

        status_entries.sort_by(|a, b| a.path.cmp(&b.path));
        unsupported_paths.sort();
        unsupported_paths.dedup();

        debug!(
            "built repo git index (subprocess + walk race): clean_count={}, status_count={}",
            ls_tree_hashes.len(),
            status_entries.len(),
        );

        Ok(Self {
            ls_tree_hashes,
            status_entries,
            unsupported_paths,
            untracked_symlinks: Vec::new(),
            untracked_entries_populated: true,
        })
    }

    #[tracing::instrument(skip(self, git))]
    pub fn populate_all_untracked(&mut self, git: &GitRepo) -> Result<(), Error> {
        self.populate_untracked(git, None)
    }

    /// Populate untracked entries from pre-walked candidate files.
    ///
    /// This is the fast path used when the filesystem walk ran in parallel
    /// with index construction. The candidates are filtered against the
    /// tracked index to identify truly untracked files.
    pub fn populate_untracked_from_candidates(&mut self, candidates: Vec<RelativeUnixPathBuf>) {
        if self.untracked_entries_populated {
            return;
        }

        let before_status_count = self.status_entries.len();
        let untracked = filter_untracked_from_candidates(
            candidates,
            &self.ls_tree_hashes,
            &self.status_entries,
        );
        for path in untracked {
            self.status_entries.push(RepoStatusEntry {
                path,
                is_delete: false,
                is_untracked: true,
            });
        }

        self.status_entries.sort_by(|a, b| a.path.cmp(&b.path));
        self.untracked_entries_populated = true;

        debug!(
            "populated repo git index from pre-walked candidates: added_count={}, status_count={}",
            self.status_entries
                .len()
                .saturating_sub(before_status_count),
            self.status_entries.len(),
        );
    }

    #[tracing::instrument(skip(self, git, prefixes))]
    pub fn populate_untracked_for_prefixes(
        &mut self,
        git: &GitRepo,
        prefixes: &[RelativeUnixPathBuf],
    ) -> Result<(), Error> {
        if prefixes.is_empty() {
            return Ok(());
        }

        self.populate_untracked(git, Some(prefixes))
    }

    fn populate_untracked(
        &mut self,
        git: &GitRepo,
        prefixes: Option<&[RelativeUnixPathBuf]>,
    ) -> Result<(), Error> {
        if self.untracked_entries_populated {
            return Ok(());
        }

        let before_status_count = self.status_entries.len();
        let untracked =
            find_untracked_files(git, &self.ls_tree_hashes, &self.status_entries, prefixes)?;
        self.unsupported_paths.extend(untracked.unsupported_paths);
        self.unsupported_paths.sort();
        self.unsupported_paths.dedup();
        for path in untracked.paths {
            self.status_entries.push(RepoStatusEntry {
                path,
                is_delete: false,
                is_untracked: true,
            });
        }
        self.untracked_symlinks = untracked.symlink_paths;
        self.untracked_symlinks.sort();

        self.status_entries.sort_by(|a, b| a.path.cmp(&b.path));
        self.untracked_entries_populated = true;

        debug!(
            "populated repo git index with untracked files: added_count={}, \
             untracked_symlinks={}, status_count={}",
            self.status_entries
                .len()
                .saturating_sub(before_status_count),
            self.untracked_symlinks.len(),
            self.status_entries.len(),
        );

        Ok(())
    }

    /// Append untracked file names from the repo index to the dirty-hash
    /// input. Returns `true` when there was any untracked entry to hash.
    ///
    /// Modified/deleted tracked files are intentionally omitted here: the diff
    /// stream is the canonical input for tracked content changes. The repo
    /// index may conservatively mark clean racy-git entries as modified so they
    /// get content-hashed later, and those must not make a clean tree dirty.
    pub fn append_dirty_status_to_hasher(&self, hasher: &mut sha2::Sha256) -> bool {
        use sha2::Digest;

        let mut has_untracked = false;
        for entry in &self.status_entries {
            if !entry.is_untracked {
                continue;
            }
            has_untracked = true;
            hasher.update(b"?\0");
            hasher.update(entry.path.as_str().as_bytes());
            hasher.update(b"\0");
        }
        for path in &self.untracked_symlinks {
            has_untracked = true;
            hasher.update(b"?\0");
            hasher.update(path.as_str().as_bytes());
            hasher.update(b"\0");
        }

        has_untracked
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
        if let Some(path) = self
            .unsupported_paths
            .iter()
            .find(|path| path.is_within_prefix(pkg_prefix))
        {
            return Err(path.clone().into_error());
        }

        let prefix_str = pkg_prefix.as_str();
        let prefix_is_empty = prefix_str.is_empty();

        // Compute range bounds once for both ls_tree and status lookups
        let range_start;
        let range_end;
        if !prefix_is_empty {
            range_start = format!("{}/", prefix_str);
            range_end = format!("{}0", prefix_str);
        } else {
            range_start = String::new();
            range_end = String::new();
        }

        let mut hashes = if prefix_is_empty {
            let mut h = GitHashes::with_capacity(self.ls_tree_hashes.len());
            for (path, hash) in &self.ls_tree_hashes {
                h.insert(path.clone(), *hash);
            }
            h
        } else {
            let lo = self
                .ls_tree_hashes
                .partition_point(|(k, _)| k.as_str() < range_start.as_str());
            let hi = self
                .ls_tree_hashes
                .partition_point(|(k, _)| k.as_str() < range_end.as_str());
            let mut h = GitHashes::with_capacity(hi - lo);
            for (path, hash) in &self.ls_tree_hashes[lo..hi] {
                if let Ok(stripped) = path.strip_prefix(pkg_prefix) {
                    h.insert(stripped, *hash);
                }
            }
            h
        };

        let mut to_hash = Vec::new();
        let status_entries = if prefix_is_empty {
            &self.status_entries[..]
        } else {
            let lo = self
                .status_entries
                .partition_point(|e| e.path.as_str() < range_start.as_str());
            let hi = self
                .status_entries
                .partition_point(|e| e.path.as_str() < range_end.as_str());
            &self.status_entries[lo..hi]
        };
        for entry in status_entries {
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

    /// Partition a set of existing git-root-relative file paths into:
    /// - clean tracked files whose blob OIDs can be reused immediately
    /// - files that still need content hashing because they are dirty,
    ///   untracked, or absent from the tracked index
    ///
    /// Status entries always win over ls-tree entries so modified tracked files
    /// are conservatively re-hashed instead of reusing stale blob IDs.
    pub fn partition_existing_paths_for_hashing(
        &self,
        paths: impl IntoIterator<Item = RelativeUnixPathBuf>,
    ) -> (
        Vec<(RelativeUnixPathBuf, OidHash)>,
        Vec<RelativeUnixPathBuf>,
    ) {
        let mut known_hashes = Vec::new();
        let mut to_hash = Vec::new();

        for path in paths {
            let in_status = self
                .status_entries
                .binary_search_by(|entry| entry.path.as_str().cmp(path.as_str()))
                .is_ok();
            if in_status {
                to_hash.push(path);
                continue;
            }

            match self
                .ls_tree_hashes
                .binary_search_by(|(entry_path, _)| entry_path.as_str().cmp(path.as_str()))
            {
                Ok(idx) => known_hashes.push((path, self.ls_tree_hashes[idx].1)),
                Err(_) => to_hash.push(path),
            }
        }

        (known_hashes, to_hash)
    }
}

struct WalkedPaths {
    paths: Vec<RelativeUnixPathBuf>,
    /// Untracked symlinks. Kept separate from `paths` because per-package
    /// hashing intentionally ignores symlinks, while dirty-hash provenance
    /// must still account for them (git treats symlinks as trackable).
    symlink_paths: Vec<RelativeUnixPathBuf>,
    unsupported_paths: Vec<UnsupportedGitPath>,
}

impl WalkedPaths {
    fn new() -> Self {
        Self {
            paths: Vec::new(),
            symlink_paths: Vec::new(),
            unsupported_paths: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.symlink_paths.is_empty() && self.unsupported_paths.is_empty()
    }
}

/// Walk the working tree to collect candidate files (all non-gitignored
/// files within scope). This is the I/O-bound phase that can run without
/// the git index.
///
/// Uses the `ignore` crate's native gitignore support to read .gitignore
/// files from disk as the walker descends. This matches standard git
/// behavior and handles tracked, untracked, and nested .gitignore files.
///
/// Returns all candidate paths relative to `git_root`. The caller must
/// filter these against the tracked index to identify truly untracked files.
#[tracing::instrument(skip(git_root, prefixes))]
pub fn walk_candidate_files(
    git_root: &std::path::Path,
    prefixes: Option<&[RelativeUnixPathBuf]>,
) -> Result<Vec<RelativeUnixPathBuf>, Error> {
    let walked = walk_candidate_files_with_unsupported(git_root, prefixes)?;
    if let Some(path) = walked.unsupported_paths.into_iter().next() {
        return Err(path.into_error());
    }

    Ok(walked.paths)
}

fn walk_candidate_files_with_unsupported(
    git_root: &std::path::Path,
    prefixes: Option<&[RelativeUnixPathBuf]>,
) -> Result<WalkedPaths, Error> {
    use std::sync::mpsc;

    use ignore::WalkBuilder;

    let root = std::sync::Arc::new(git_root.to_path_buf());
    let scope = std::sync::Arc::new(UntrackedScope::new(prefixes));

    let (tx, rx) = mpsc::channel::<WalkedPaths>();

    let walker = WalkBuilder::new(root.as_path())
        .follow_links(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .ignore(false)
        .parents(false)
        .hidden(false)
        .filter_entry({
            let root = root.clone();
            let scope = scope.clone();
            move |entry| {
                if entry.file_name() == ".git" {
                    return false;
                }
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                let path = entry.path();

                let rel_path = match path.strip_prefix(root.as_path()) {
                    Ok(rel) => rel,
                    Err(_) => return false,
                };
                let rel_path = path_to_git_path_bytes(rel_path);

                if is_dir {
                    scope.should_visit_dir_bytes(rel_path.as_ref())
                } else {
                    scope.should_consider_file_bytes(
                        rel_path.as_ref(),
                        entry.file_name() == ".gitignore",
                    )
                }
            }
        })
        .threads(rayon::current_num_threads().min(8))
        .build_parallel();

    struct FlushOnDrop {
        batch: WalkedPaths,
        tx: mpsc::Sender<WalkedPaths>,
    }

    impl Drop for FlushOnDrop {
        fn drop(&mut self) {
            if !self.batch.is_empty() {
                let batch = WalkedPaths {
                    paths: std::mem::take(&mut self.batch.paths),
                    symlink_paths: std::mem::take(&mut self.batch.symlink_paths),
                    unsupported_paths: std::mem::take(&mut self.batch.unsupported_paths),
                };
                let _ = self.tx.send(batch);
            }
        }
    }

    walker.run(|| {
        let root = root.clone();
        let mut guard = FlushOnDrop {
            batch: WalkedPaths::new(),
            tx: tx.clone(),
        };

        Box::new(move |entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };

            // Skip anything that isn't a regular file (directories,
            // symlinks, sockets, FIFOs, device nodes).
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return ignore::WalkState::Continue;
            }

            let abs_path = entry.into_path();
            let rel_path = match abs_path.strip_prefix(root.as_path()) {
                Ok(rel) => rel,
                Err(_) => return ignore::WalkState::Continue,
            };

            match parse_path(rel_path, "working tree path") {
                Ok(Ok(path)) => guard.batch.paths.push(path),
                Ok(Err(path)) => guard.batch.unsupported_paths.push(path),
                Err(_) => {}
            }

            ignore::WalkState::Continue
        })
    });
    drop(tx);

    let mut candidates = WalkedPaths::new();
    for batch in rx.iter() {
        candidates.paths.extend(batch.paths);
        candidates.unsupported_paths.extend(batch.unsupported_paths);
    }

    Ok(candidates)
}

/// Filter pre-walked candidate files against the git index to identify
/// truly untracked files. This is the CPU-bound phase that runs after
/// the tracked index is ready.
fn filter_untracked_from_candidates(
    candidates: Vec<RelativeUnixPathBuf>,
    ls_tree_hashes: &SortedGitHashes,
    status_entries: &[RepoStatusEntry],
) -> Vec<RelativeUnixPathBuf> {
    candidates
        .into_iter()
        .filter(|path| {
            let s = path.as_str();
            let in_ls_tree = ls_tree_hashes
                .binary_search_by(|(p, _)| p.as_str().cmp(s))
                .is_ok();
            let in_status = status_entries
                .binary_search_by(|e| e.path.as_str().cmp(s))
                .is_ok();
            !in_ls_tree && !in_status
        })
        .collect()
}

/// Walk the working tree to find untracked files (files on disk that are
/// not in the git index). Uses the `ignore` crate's parallel walker to
/// respect .gitignore rules. Binary searches directly on the sorted
/// `ls_tree_hashes` and `status_entries` slices — no intermediate
/// allocations needed.
///
/// When `prefixes` is provided, the walker prunes subtrees outside the
/// requested package prefixes while still visiting ancestor `.gitignore`
/// files that can affect those prefixes.
///
/// IMPORTANT: `status_entries` must be sorted by path before calling.
///
/// Each walker thread accumulates results in a thread-local Vec and
/// batch-sends them through a channel, avoiding per-file mutex contention.
#[tracing::instrument(skip(git, ls_tree_hashes, status_entries, prefixes))]
fn find_untracked_files(
    git: &GitRepo,
    ls_tree_hashes: &SortedGitHashes,
    status_entries: &[RepoStatusEntry],
    prefixes: Option<&[RelativeUnixPathBuf]>,
) -> Result<WalkedPaths, Error> {
    use std::{collections::HashMap, sync::mpsc};

    use ignore::WalkBuilder;

    let root = std::sync::Arc::new(git.root.as_std_path().to_path_buf());
    let scope = std::sync::Arc::new(UntrackedScope::new(prefixes));

    // Pre-build gitignore matchers from all tracked .gitignore files.
    // Each .gitignore is built with a builder rooted at its containing
    // directory so patterns are scoped correctly (e.g., `dist/` in
    // `packages/ui/.gitignore` only matches under `packages/ui/`).
    let gitignore_matchers = {
        let mut matchers: HashMap<std::path::PathBuf, ignore::gitignore::Gitignore> =
            HashMap::new();

        // Global gitignore + .git/info/exclude are rooted at the repo root
        let mut root_builder = ignore::gitignore::GitignoreBuilder::new(root.as_path());
        let mut has_root_rules = false;
        if let Some(global_path) = ignore::gitignore::gitconfig_excludes_path()
            && global_path.exists()
        {
            let _ = root_builder.add(&global_path);
            has_root_rules = true;
        }
        let info_exclude = root.join(".git").join("info").join("exclude");
        if info_exclude.exists() {
            let _ = root_builder.add(&info_exclude);
            has_root_rules = true;
        }

        // Collect .gitignore paths from both clean tracked files and
        // dirty (modified-but-tracked) files. A modified .gitignore
        // lands in status_entries instead of ls_tree_hashes, but its
        // on-disk patterns must still be respected.
        let gitignore_paths: Vec<&str> = ls_tree_hashes
            .iter()
            .map(|(p, _)| p.as_str())
            .chain(
                status_entries
                    .iter()
                    .filter(|e| !e.is_delete)
                    .map(|e| e.path.as_str()),
            )
            .filter(|s| s.ends_with(".gitignore"))
            .collect();

        for s in gitignore_paths {
            let abs_path = root.join(s);
            if !abs_path.exists() {
                continue;
            }
            let gi_dir = abs_path.parent().unwrap_or(root.as_path());
            if gi_dir == root.as_path() {
                // Root .gitignore goes into the root builder alongside
                // global and info/exclude rules
                let _ = root_builder.add(&abs_path);
                has_root_rules = true;
            } else {
                // Nested .gitignore gets its own matcher scoped to its dir
                let mut builder = ignore::gitignore::GitignoreBuilder::new(gi_dir);
                let _ = builder.add(&abs_path);
                if let Ok(gi) = builder.build()
                    && !gi.is_empty()
                {
                    matchers.insert(gi_dir.to_path_buf(), gi);
                }
            }
        }

        if has_root_rules
            && let Ok(gi) = root_builder.build()
            && !gi.is_empty()
        {
            matchers.insert(root.as_path().to_path_buf(), gi);
        }

        matchers
    };
    let gitignore_matchers = std::sync::Arc::new(gitignore_matchers);

    let (tx, rx) = mpsc::channel::<WalkedPaths>();

    // Disable ALL per-directory probing. Gitignore rules are applied via
    // filter_entry using the pre-built matcher above.
    let walker = WalkBuilder::new(root.as_path())
        .follow_links(false)
        .git_ignore(false)
        .git_exclude(false)
        .require_git(false)
        .ignore(false)
        .parents(false)
        .hidden(false)
        .filter_entry({
            let matchers = gitignore_matchers.clone();
            let root = root.clone();
            let scope = scope.clone();
            move |entry| {
                if entry.file_name() == ".git" {
                    return false;
                }
                let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
                let path = entry.path();

                let rel_path = match path.strip_prefix(root.as_path()) {
                    Ok(rel) => rel,
                    Err(_) => return false,
                };
                let rel_path = path_to_git_path_bytes(rel_path);

                let in_scope = if is_dir {
                    scope.should_visit_dir_bytes(rel_path.as_ref())
                } else {
                    scope.should_consider_file_bytes(
                        rel_path.as_ref(),
                        entry.file_name() == ".gitignore",
                    )
                };
                if !in_scope {
                    return false;
                }

                // Only ancestor .gitignore files can affect this entry.
                !is_ignored_by_indexed_gitignore_matchers(&matchers, root.as_path(), path, is_dir)
            }
        })
        .threads(rayon::current_num_threads().min(8))
        .build_parallel();

    struct FlushOnDrop {
        batch: WalkedPaths,
        tx: mpsc::Sender<WalkedPaths>,
    }

    impl Drop for FlushOnDrop {
        fn drop(&mut self) {
            if !self.batch.is_empty() {
                let batch = WalkedPaths {
                    paths: std::mem::take(&mut self.batch.paths),
                    symlink_paths: std::mem::take(&mut self.batch.symlink_paths),
                    unsupported_paths: std::mem::take(&mut self.batch.unsupported_paths),
                };
                let _ = self.tx.send(batch);
            }
        }
    }

    walker.run(|| {
        let root = root.clone();
        let mut guard = FlushOnDrop {
            batch: WalkedPaths::new(),
            tx: tx.clone(),
        };

        Box::new(move |entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };

            // Regular files feed per-package hashing. Symlinks are trackable
            // by git and so count as untracked working-tree state, but are
            // collected separately so hashing never sees them. Everything
            // else (directories, sockets, FIFOs, device nodes) is skipped.
            let file_type = entry.file_type();
            let is_file = file_type.as_ref().is_some_and(|ft| ft.is_file());
            let is_symlink = file_type.is_some_and(|ft| ft.is_symlink());
            if !is_file && !is_symlink {
                return ignore::WalkState::Continue;
            }

            let abs_path = entry.into_path();
            let rel_path = match abs_path.strip_prefix(root.as_path()) {
                Ok(rel) => rel,
                Err(_) => return ignore::WalkState::Continue,
            };

            let path = match parse_path(rel_path, "working tree path") {
                Ok(Ok(path)) => path,
                Ok(Err(path)) => {
                    guard.batch.unsupported_paths.push(path);
                    return ignore::WalkState::Continue;
                }
                Err(_) => return ignore::WalkState::Continue,
            };

            let unix_str = path.as_str();
            let in_ls_tree = ls_tree_hashes
                .binary_search_by(|(p, _)| p.as_str().cmp(unix_str))
                .is_ok();
            let in_status = status_entries
                .binary_search_by(|e| e.path.as_str().cmp(unix_str))
                .is_ok();

            if !in_ls_tree && !in_status {
                if is_file {
                    guard.batch.paths.push(path);
                } else {
                    guard.batch.symlink_paths.push(path);
                }
            }

            ignore::WalkState::Continue
        })
    });
    drop(tx);

    let mut untracked = WalkedPaths::new();
    for batch in rx.iter() {
        untracked.paths.extend(batch.paths);
        untracked.symlink_paths.extend(batch.symlink_paths);
        untracked.unsupported_paths.extend(batch.unsupported_paths);
    }

    // Post-filter: check for untracked .gitignore files that we couldn't
    // know about during the walk. If any exist, build per-directory matchers
    // from them and remove files that should be ignored.
    let untracked_gitignores: Vec<&RelativeUnixPathBuf> = untracked
        .paths
        .iter()
        .filter(|p| p.as_str().ends_with(".gitignore"))
        .collect();

    if !untracked_gitignores.is_empty() {
        let mut extra_matchers: HashMap<std::path::PathBuf, ignore::gitignore::Gitignore> =
            HashMap::new();
        for gi_path in &untracked_gitignores {
            let abs = root.join(gi_path.as_str());
            let gi_dir = abs.parent().unwrap_or(root.as_path());
            let mut builder = ignore::gitignore::GitignoreBuilder::new(gi_dir);
            let _ = builder.add(&abs);
            if let Ok(gi) = builder.build()
                && !gi.is_empty()
            {
                extra_matchers.insert(gi_dir.to_path_buf(), gi);
            }
        }
        if !extra_matchers.is_empty() {
            let not_ignored = |p: &RelativeUnixPathBuf| {
                if p.as_str().ends_with(".gitignore") {
                    return true;
                }
                let abs = root.join(p.as_str());
                !is_ignored_by_indexed_gitignore_matchers(
                    &extra_matchers,
                    root.as_path(),
                    &abs,
                    false,
                )
            };
            untracked.paths.retain(not_ignored);
            untracked.symlink_paths.retain(not_ignored);
        }
    }

    Ok(untracked)
}

#[derive(Debug, Clone)]
struct UntrackedScope {
    prefixes: Vec<String>,
    is_full_walk: bool,
}

impl UntrackedScope {
    fn new(prefixes: Option<&[RelativeUnixPathBuf]>) -> Self {
        let Some(prefixes) = prefixes else {
            return Self {
                prefixes: Vec::new(),
                is_full_walk: true,
            };
        };

        if prefixes.iter().any(|prefix| prefix.as_str().is_empty()) {
            return Self {
                prefixes: Vec::new(),
                is_full_walk: true,
            };
        }

        let mut normalized = prefixes
            .iter()
            .map(|prefix| prefix.as_str().to_string())
            .collect::<Vec<_>>();
        normalized.sort_unstable();
        normalized.dedup();

        let mut scoped: Vec<String> = Vec::with_capacity(normalized.len());
        for prefix in normalized {
            if scoped
                .iter()
                .any(|existing| prefix == *existing || is_nested_path(&prefix, existing))
            {
                continue;
            }
            scoped.push(prefix);
        }

        Self {
            prefixes: scoped,
            is_full_walk: false,
        }
    }

    #[cfg(test)]
    fn should_visit_dir(&self, rel_path: &str) -> bool {
        self.should_visit_dir_bytes(rel_path.as_bytes())
    }

    fn should_visit_dir_bytes(&self, rel_path: &[u8]) -> bool {
        if self.is_full_walk || rel_path.is_empty() {
            return true;
        }

        self.prefixes.iter().any(|prefix| {
            let prefix = prefix.as_bytes();
            rel_path == prefix
                || is_nested_path_bytes(rel_path, prefix)
                || is_nested_path_bytes(prefix, rel_path)
        })
    }

    #[cfg(test)]
    fn should_consider_file(&self, rel_path: &str, is_gitignore: bool) -> bool {
        self.should_consider_file_bytes(rel_path.as_bytes(), is_gitignore)
    }

    fn should_consider_file_bytes(&self, rel_path: &[u8], is_gitignore: bool) -> bool {
        if self.is_full_walk || self.is_within_selected_prefix_bytes(rel_path) {
            return true;
        }

        is_gitignore && self.should_visit_dir_bytes(parent_path_bytes(rel_path))
    }

    #[cfg(test)]
    fn is_within_selected_prefix(&self, rel_path: &str) -> bool {
        self.is_within_selected_prefix_bytes(rel_path.as_bytes())
    }

    fn is_within_selected_prefix_bytes(&self, rel_path: &[u8]) -> bool {
        if self.is_full_walk {
            return true;
        }

        self.prefixes.iter().any(|prefix| {
            let prefix = prefix.as_bytes();
            rel_path == prefix || is_nested_path_bytes(rel_path, prefix)
        })
    }
}

fn is_nested_path(path: &str, prefix: &str) -> bool {
    is_nested_path_bytes(path.as_bytes(), prefix.as_bytes())
}

fn is_nested_path_bytes(path: &[u8], prefix: &[u8]) -> bool {
    path.len() > prefix.len() && path.starts_with(prefix) && path.get(prefix.len()) == Some(&b'/')
}

fn is_ignored_by_indexed_gitignore_matchers(
    matchers: &std::collections::HashMap<std::path::PathBuf, ignore::gitignore::Gitignore>,
    root: &std::path::Path,
    path: &std::path::Path,
    is_dir: bool,
) -> bool {
    let mut matcher_dir = if is_dir {
        path
    } else {
        path.parent().unwrap_or(root)
    };

    loop {
        if let Some(matcher) = matchers.get(matcher_dir)
            && matcher
                .matched_path_or_any_parents(path, is_dir)
                .is_ignore()
        {
            return true;
        }

        if matcher_dir == root {
            break;
        }

        let Some(parent) = matcher_dir.parent() else {
            break;
        };
        matcher_dir = parent;
    }

    false
}

fn parent_path_bytes(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/')
        .map(|idx| &path[..idx])
        .unwrap_or(b"")
}

enum EntryClassification {
    Clean {
        path: RelativeUnixPathBuf,
        oid: OidHash,
    },
    Modified {
        path: RelativeUnixPathBuf,
    },
    Deleted {
        path: RelativeUnixPathBuf,
    },
    Unsupported(UnsupportedGitPath),
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        sync::OnceLock,
    };

    use tempfile::TempDir;
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};

    use super::*;

    fn path(s: &str) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf::new(s).unwrap()
    }

    fn pad_hex(s: &str) -> String {
        format!("{:0<40}", s)
    }

    fn make_index(ls_tree: Vec<(&str, &str)>, status: Vec<(&str, bool)>) -> RepoGitIndex {
        let mut ls_tree_hashes: SortedGitHashes = ls_tree
            .into_iter()
            .map(|(p, h)| (path(p), OidHash::from_hex_str(&pad_hex(h))))
            .collect::<Vec<_>>();
        ls_tree_hashes.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut status_entries: Vec<RepoStatusEntry> = status
            .into_iter()
            .map(|(p, is_delete)| RepoStatusEntry {
                path: path(p),
                is_delete,
                is_untracked: false,
            })
            .collect();
        status_entries.sort_by(|a, b| a.path.cmp(&b.path));
        RepoGitIndex {
            ls_tree_hashes,
            status_entries,
            unsupported_paths: Vec::new(),
            untracked_symlinks: Vec::new(),
            untracked_entries_populated: true,
        }
    }

    fn make_unpopulated_index(
        ls_tree: Vec<(&str, &str)>,
        status: Vec<(&str, bool)>,
    ) -> RepoGitIndex {
        let mut index = make_index(ls_tree, status);
        index.untracked_entries_populated = false;
        index
    }

    fn write_file(root: &std::path::Path, rel_path: &str, contents: &str) {
        let full_path = root.join(rel_path);
        std::fs::create_dir_all(full_path.parent().unwrap()).unwrap();
        std::fs::write(full_path, contents).unwrap();
    }

    fn test_git_repo(root: &std::path::Path) -> GitRepo {
        let root = AbsoluteSystemPathBuf::try_from(root).unwrap();
        GitRepo {
            root: root.clone(),
            bin: root,
            attrs: OnceLock::new(),
            slowest_files: None,
        }
    }

    fn add_gitignore(
        root: &std::path::Path,
        dir: &str,
        patterns: &str,
        matchers: &mut HashMap<std::path::PathBuf, ignore::gitignore::Gitignore>,
    ) {
        let dir = root.join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gitignore = dir.join(".gitignore");
        std::fs::write(&gitignore, patterns).unwrap();

        let mut builder = ignore::gitignore::GitignoreBuilder::new(&dir);
        builder.add(&gitignore);
        matchers.insert(dir, builder.build().unwrap());
    }

    #[test]
    fn test_empty_prefix_returns_all_files() {
        let index = make_index(
            vec![
                ("apps/web/src/index.ts", "aaa"),
                ("packages/ui/button.tsx", "bbb"),
                ("root-file.json", "ccc"),
            ],
            vec![],
        );
        let (hashes, to_hash) = index.get_package_hashes(&path("")).unwrap();
        assert_eq!(hashes.len(), 3);
        assert!(to_hash.is_empty());
    }

    #[test]
    fn test_prefix_filters_to_package_and_strips_prefix() {
        let index = make_index(
            vec![
                ("apps/web/src/index.ts", "aaa"),
                ("apps/web/package.json", "bbb"),
                ("apps/docs/README.md", "ccc"),
                ("packages/ui/button.tsx", "ddd"),
            ],
            vec![],
        );
        let (hashes, to_hash) = index.get_package_hashes(&path("apps/web")).unwrap();
        assert_eq!(hashes.len(), 2);
        assert_eq!(*hashes.get(&path("src/index.ts")).unwrap(), *pad_hex("aaa"));
        assert_eq!(*hashes.get(&path("package.json")).unwrap(), *pad_hex("bbb"));
        assert!(to_hash.is_empty());
    }

    #[test]
    fn test_prefix_does_not_match_sibling_with_shared_prefix() {
        let index = make_index(
            vec![
                ("apps/web/index.ts", "aaa"),
                ("apps/web-admin/index.ts", "bbb"),
            ],
            vec![],
        );
        let (hashes, _) = index.get_package_hashes(&path("apps/web")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert!(hashes.contains_key(&path("index.ts")));
    }

    #[test]
    fn test_status_modified_file_added_to_to_hash() {
        let index = make_index(
            vec![("my-pkg/file.ts", "aaa")],
            vec![("my-pkg/new-file.ts", false)],
        );
        let (hashes, to_hash) = index.get_package_hashes(&path("my-pkg")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(to_hash, vec![path("my-pkg/new-file.ts")]);
    }

    #[test]
    fn test_status_deleted_file_removed_from_hashes() {
        let index = make_index(
            vec![("my-pkg/keep.ts", "aaa"), ("my-pkg/deleted.ts", "bbb")],
            vec![("my-pkg/deleted.ts", true)],
        );
        let (hashes, to_hash) = index.get_package_hashes(&path("my-pkg")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert!(hashes.contains_key(&path("keep.ts")));
        assert!(to_hash.is_empty());
    }

    #[test]
    fn test_status_entries_for_other_packages_ignored() {
        let index = make_index(
            vec![("pkg-a/file.ts", "aaa")],
            vec![("pkg-b/new.ts", false), ("pkg-b/gone.ts", true)],
        );
        let (hashes, to_hash) = index.get_package_hashes(&path("pkg-a")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert!(to_hash.is_empty());
    }

    #[test]
    fn test_empty_prefix_with_status() {
        let index = make_index(
            vec![("file.ts", "aaa")],
            vec![("new.ts", false), ("file.ts", true)],
        );
        let (hashes, to_hash) = index.get_package_hashes(&path("")).unwrap();
        assert!(hashes.is_empty());
        assert_eq!(to_hash, vec![path("new.ts")]);
    }

    #[test]
    fn test_sorted_status_binary_search_matches_linear_scan() {
        let status = vec![
            ("apps/docs/new.ts", false),
            ("apps/web/changed.ts", false),
            ("apps/web-admin/added.ts", false),
            ("apps/web/deleted.ts", true),
            ("packages/ui/modified.ts", false),
            ("root-new.ts", false),
        ];
        let index = make_index(
            vec![
                ("apps/docs/index.ts", "aaa"),
                ("apps/web/index.ts", "bbb"),
                ("apps/web/deleted.ts", "ccc"),
                ("apps/web-admin/index.ts", "ddd"),
                ("packages/ui/button.tsx", "eee"),
            ],
            status,
        );

        let (hashes, to_hash) = index.get_package_hashes(&path("apps/web")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert!(hashes.contains_key(&path("index.ts")));
        assert_eq!(to_hash, vec![path("apps/web/changed.ts")]);

        let (_, to_hash) = index.get_package_hashes(&path("apps/web-admin")).unwrap();
        assert_eq!(to_hash, vec![path("apps/web-admin/added.ts")]);

        let (_, to_hash) = index.get_package_hashes(&path("")).unwrap();
        assert_eq!(to_hash.len(), 5);
    }

    #[test]
    fn test_range_query_equivalence_with_binary_search() {
        let ls_tree_data = vec![
            ("apps/docs/README.md", "aaa"),
            ("apps/docs/package.json", "bbb"),
            ("apps/web-admin/index.ts", "ccc"),
            ("apps/web/package.json", "ddd"),
            ("apps/web/src/index.ts", "eee"),
            ("apps/web/src/utils.ts", "fff"),
            ("packages/ui/button.tsx", "111"),
            ("packages/ui/package.json", "222"),
            ("root.json", "333"),
        ];
        let index = make_index(ls_tree_data.clone(), vec![]);

        let (hashes, _) = index.get_package_hashes(&path("apps/web")).unwrap();
        assert_eq!(hashes.len(), 3);

        let (hashes, _) = index.get_package_hashes(&path("apps/docs")).unwrap();
        assert_eq!(hashes.len(), 2);

        let (hashes, _) = index.get_package_hashes(&path("packages/ui")).unwrap();
        assert_eq!(hashes.len(), 2);

        let (hashes, _) = index.get_package_hashes(&path("nonexistent")).unwrap();
        assert_eq!(hashes.len(), 0);

        let sorted_vec: Vec<(RelativeUnixPathBuf, String)> = ls_tree_data
            .iter()
            .map(|(p, h)| (path(p), h.to_string()))
            .collect();
        assert!(sorted_vec.windows(2).all(|w| w[0].0 < w[1].0));

        let prefix = "apps/web";
        let range_start = path(&format!("{prefix}/"));
        let range_end = path(&format!("{prefix}0"));
        let lo = sorted_vec.partition_point(|(k, _)| *k < range_start);
        let hi = sorted_vec.partition_point(|(k, _)| *k < range_end);
        let vec_results: Vec<_> = sorted_vec[lo..hi]
            .iter()
            .map(|(p, h)| (p.clone(), h.clone()))
            .collect();

        let btree: BTreeMap<RelativeUnixPathBuf, String> = ls_tree_data
            .iter()
            .map(|(p, h)| (path(p), h.to_string()))
            .collect();
        let btree_results: Vec<_> = btree
            .range(range_start..range_end)
            .map(|(p, h)| (p.clone(), h.clone()))
            .collect();
        assert_eq!(vec_results, btree_results);
    }

    #[test]
    fn test_full_copy_preserves_all_entries() {
        let ls_tree_data = vec![("a.ts", "111"), ("b/c.ts", "222"), ("d/e/f.ts", "333")];
        let index = make_index(ls_tree_data, vec![]);
        let (hashes, to_hash) = index.get_package_hashes(&path("")).unwrap();
        assert_eq!(hashes.len(), 3);
        assert_eq!(*hashes.get(&path("a.ts")).unwrap(), *pad_hex("111"));
        assert!(to_hash.is_empty());
    }

    #[test]
    fn test_indexed_gitignore_matchers_consult_ancestors_only() {
        let tempdir = TempDir::new().unwrap();
        let root = tempdir.path();
        let mut matchers = HashMap::new();

        add_gitignore(root, "", "root-ignore\n", &mut matchers);
        add_gitignore(root, "packages/ui", "dist/\n", &mut matchers);
        add_gitignore(root, "apps/web", "dist/\n", &mut matchers);

        assert!(is_ignored_by_indexed_gitignore_matchers(
            &matchers,
            root,
            &root.join("root-ignore"),
            false,
        ));
        assert!(is_ignored_by_indexed_gitignore_matchers(
            &matchers,
            root,
            &root.join("packages/ui/dist/index.js"),
            false,
        ));
        assert!(!is_ignored_by_indexed_gitignore_matchers(
            &matchers,
            root,
            &root.join("packages/core/dist/index.js"),
            false,
        ));
    }

    #[test]
    fn test_walk_candidate_files_respects_prefixes_gitignore_and_empty_dirs() {
        let tempdir = TempDir::new().unwrap();
        let root = tempdir.path();

        write_file(root, ".gitignore", "*.log\nnode_modules/\n");
        write_file(root, "packages/ui/.gitignore", "output/\n");
        write_file(root, "packages/ui/src/button.tsx", "button");
        write_file(root, "packages/ui/output/bundle.js", "ignored ui output");
        write_file(root, "apps/web/output/bundle.js", "web output");
        write_file(root, "apps/web/debug.log", "ignored log");
        write_file(root, "packages/core/index.ts", "outside prefix");
        std::fs::create_dir_all(root.join("packages/ui/empty")).unwrap();

        let prefixes = [path("packages/ui"), path("apps/web")];
        let mut candidates = walk_candidate_files(root, Some(&prefixes)).unwrap();
        candidates.sort();

        assert_eq!(
            candidates,
            vec![
                path(".gitignore"),
                path("apps/web/output/bundle.js"),
                path("packages/ui/.gitignore"),
                path("packages/ui/src/button.tsx"),
            ]
        );
    }

    #[test]
    fn test_find_untracked_files_uses_dirty_gitignore_status_entries() {
        let tempdir = TempDir::new().unwrap();
        let root = tempdir.path();
        let git = test_git_repo(root);

        write_file(root, ".gitignore", "node_modules/\n");
        write_file(root, "pkg-a/.gitignore", "dist/\n");
        write_file(root, "pkg-a/src/index.ts", "tracked");
        write_file(root, "pkg-a/package.json", "{}");
        write_file(root, "pkg-a/keep.ts", "untracked");
        write_file(root, "pkg-a/node_modules/dep/index.js", "ignored by root");
        write_file(root, "pkg-a/dist/out.js", "ignored by nested");

        let index = make_index(
            vec![("pkg-a/package.json", "aaa"), ("pkg-a/src/index.ts", "bbb")],
            vec![(".gitignore", false), ("pkg-a/.gitignore", false)],
        );

        let prefixes = [path("pkg-a")];
        let mut untracked = find_untracked_files(
            &git,
            &index.ls_tree_hashes,
            &index.status_entries,
            Some(&prefixes),
        )
        .unwrap()
        .paths;
        untracked.sort();

        assert_eq!(untracked, vec![path("pkg-a/keep.ts")]);
    }

    #[test]
    fn test_status_binary_search_matches_linear_scan() {
        let index = make_index(
            vec![
                ("apps/docs/README.md", "aaa"),
                ("apps/web-admin/index.ts", "bbb"),
                ("apps/web/index.ts", "ccc"),
                ("apps/web/lib.ts", "ddd"),
                ("packages/ui/button.tsx", "eee"),
            ],
            vec![
                ("apps/docs/new-doc.md", false),
                ("apps/web-admin/deleted.ts", true),
                ("apps/web/dirty.ts", false),
                ("apps/web/index.ts", true),
                ("packages/ui/new-component.tsx", false),
                ("root-level-file.ts", false),
            ],
        );

        let (hashes, to_hash) = index.get_package_hashes(&path("apps/web")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(to_hash, vec![path("apps/web/dirty.ts")]);

        let (hashes, to_hash) = index.get_package_hashes(&path("apps/web-admin")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert!(to_hash.is_empty());

        let (hashes, to_hash) = index.get_package_hashes(&path("apps/docs")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(to_hash, vec![path("apps/docs/new-doc.md")]);

        let (hashes, to_hash) = index.get_package_hashes(&path("packages/ui")).unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(to_hash, vec![path("packages/ui/new-component.tsx")]);

        let (hashes, to_hash) = index.get_package_hashes(&path("")).unwrap();
        assert_eq!(hashes.len(), 4);
        assert_eq!(to_hash.len(), 4);
    }

    #[test]
    fn test_status_substring_prefix_not_matched() {
        let index = make_index(
            vec![("pkg/file.ts", "aaa"), ("pkg-extra/file.ts", "bbb")],
            vec![("pkg-extra/dirty.ts", false), ("pkg/dirty.ts", false)],
        );

        let (_, to_hash) = index.get_package_hashes(&path("pkg")).unwrap();
        assert_eq!(to_hash, vec![path("pkg/dirty.ts")]);

        let (_, to_hash) = index.get_package_hashes(&path("pkg-extra")).unwrap();
        assert_eq!(to_hash, vec![path("pkg-extra/dirty.ts")]);
    }

    #[test]
    fn test_status_binary_search_empty_status() {
        let index = make_index(vec![("pkg/a.ts", "aaa"), ("pkg/b.ts", "bbb")], vec![]);
        let (hashes, to_hash) = index.get_package_hashes(&path("pkg")).unwrap();
        assert_eq!(hashes.len(), 2);
        assert!(to_hash.is_empty());
    }

    #[test]
    fn test_status_all_entries_same_package() {
        let index = make_index(
            vec![("pkg/a.ts", "aaa")],
            vec![("pkg/b.ts", false), ("pkg/c.ts", false), ("pkg/a.ts", true)],
        );
        let (hashes, to_hash) = index.get_package_hashes(&path("pkg")).unwrap();
        assert!(hashes.is_empty(), "a.ts was deleted");
        assert_eq!(to_hash, vec![path("pkg/b.ts"), path("pkg/c.ts")]);
    }

    // UntrackedScope tests

    #[test]
    fn test_untracked_scope_none_prefixes_is_full_walk() {
        let scope = UntrackedScope::new(None);
        assert!(scope.is_full_walk);
        assert!(scope.should_visit_dir("anything"));
        assert!(scope.should_consider_file("any/file.ts", false));
        assert!(scope.is_within_selected_prefix("any/path"));
    }

    #[test]
    fn test_untracked_scope_empty_string_prefix_is_full_walk() {
        let prefixes = [path("")];
        let scope = UntrackedScope::new(Some(&prefixes));
        assert!(scope.is_full_walk);
        assert!(scope.should_visit_dir("packages/a"));
        assert!(scope.should_consider_file("packages/a/file.ts", false));
    }

    #[test]
    fn test_untracked_scope_nested_prefixes_deduplicated() {
        let prefixes = [path("packages/a"), path("packages/a/sub")];
        let scope = UntrackedScope::new(Some(&prefixes));
        assert!(!scope.is_full_walk);
        // packages/a/sub is nested under packages/a, so only packages/a should remain
        assert_eq!(scope.prefixes.len(), 1);
        assert_eq!(scope.prefixes[0], "packages/a");
    }

    #[test]
    fn test_untracked_scope_duplicate_prefixes_deduplicated() {
        let prefixes = [path("packages/a"), path("packages/b"), path("packages/a")];
        let scope = UntrackedScope::new(Some(&prefixes));
        assert_eq!(scope.prefixes.len(), 2);
    }

    #[test]
    fn test_untracked_scope_should_visit_dir_ancestor_traversal() {
        let prefixes = [path("packages/a")];
        let scope = UntrackedScope::new(Some(&prefixes));

        // Ancestor dirs must be visited to reach the prefix
        assert!(scope.should_visit_dir(""), "root dir");
        assert!(scope.should_visit_dir("packages"), "parent of prefix");
        assert!(scope.should_visit_dir("packages/a"), "exact prefix");
        assert!(scope.should_visit_dir("packages/a/src"), "child of prefix");
    }

    #[test]
    fn test_untracked_scope_should_visit_dir_sibling_excluded() {
        let prefixes = [path("packages/a")];
        let scope = UntrackedScope::new(Some(&prefixes));

        assert!(!scope.should_visit_dir("packages/b"), "sibling excluded");
        assert!(!scope.should_visit_dir("apps"), "unrelated dir excluded");
        assert!(
            !scope.should_visit_dir("packages/ab"),
            "substring prefix not matched"
        );
    }

    #[test]
    fn test_untracked_scope_should_consider_file_within_prefix() {
        let prefixes = [path("packages/a")];
        let scope = UntrackedScope::new(Some(&prefixes));

        assert!(scope.should_consider_file("packages/a/file.ts", false));
        assert!(scope.should_consider_file("packages/a/src/deep.ts", false));
    }

    #[test]
    fn test_untracked_scope_should_consider_file_outside_prefix() {
        let prefixes = [path("packages/a")];
        let scope = UntrackedScope::new(Some(&prefixes));

        assert!(!scope.should_consider_file("packages/b/file.ts", false));
        assert!(!scope.should_consider_file("root.json", false));
    }

    #[test]
    fn test_untracked_scope_ancestor_gitignore_considered() {
        let prefixes = [path("packages/a")];
        let scope = UntrackedScope::new(Some(&prefixes));

        // .gitignore at repo root should be considered (ancestor of prefix)
        assert!(scope.should_consider_file(".gitignore", true));
        // .gitignore in packages/ should be considered (parent of prefix)
        assert!(scope.should_consider_file("packages/.gitignore", true));
        // .gitignore inside the prefix itself
        assert!(scope.should_consider_file("packages/a/.gitignore", true));
        // .gitignore in a sibling package should NOT be considered
        assert!(!scope.should_consider_file("packages/b/.gitignore", true));
    }

    #[test]
    fn test_untracked_scope_empty_prefixes_slice_considers_nothing() {
        let prefixes: Vec<RelativeUnixPathBuf> = vec![];
        let scope = UntrackedScope::new(Some(&prefixes));
        assert!(!scope.is_full_walk);
        // Empty prefix list means no dirs/files are in scope
        assert!(!scope.should_consider_file("any/file.ts", false));
        // Root dir is always visitable (empty string check)
        assert!(scope.should_visit_dir(""));
    }

    #[test]
    fn test_untracked_scope_multiple_disjoint_prefixes() {
        let prefixes = [path("apps/web"), path("packages/ui")];
        let scope = UntrackedScope::new(Some(&prefixes));

        assert!(scope.should_visit_dir("apps"));
        assert!(scope.should_visit_dir("apps/web"));
        assert!(scope.should_visit_dir("packages"));
        assert!(scope.should_visit_dir("packages/ui"));
        assert!(!scope.should_visit_dir("apps/docs"));
        assert!(!scope.should_visit_dir("packages/utils"));

        assert!(scope.should_consider_file("apps/web/index.ts", false));
        assert!(scope.should_consider_file("packages/ui/button.tsx", false));
        assert!(!scope.should_consider_file("apps/docs/readme.md", false));
    }

    #[test]
    fn test_partition_existing_paths_for_hashing_reuses_clean_tracked_only() {
        let index = make_index(
            vec![
                ("pkg/clean.ts", "aaa"),
                ("pkg/also-clean.ts", "bbb"),
                ("root/config.json", "ccc"),
            ],
            vec![("pkg/dirty.ts", false), ("pkg/deleted.ts", true)],
        );

        let (known_hashes, to_hash) = index.partition_existing_paths_for_hashing(vec![
            path("pkg/clean.ts"),
            path("pkg/dirty.ts"),
            path("pkg/deleted.ts"),
            path("pkg/untracked.ts"),
            path("root/config.json"),
        ]);

        assert_eq!(
            known_hashes,
            vec![
                (path("pkg/clean.ts"), OidHash::from_hex_str(&pad_hex("aaa"))),
                (
                    path("root/config.json"),
                    OidHash::from_hex_str(&pad_hex("ccc"))
                ),
            ]
        );
        assert_eq!(
            to_hash,
            vec![
                path("pkg/dirty.ts"),
                path("pkg/deleted.ts"),
                path("pkg/untracked.ts"),
            ]
        );
    }

    #[test]
    fn test_populate_untracked_from_candidates_filters_known_paths_and_is_idempotent() {
        let mut index = make_unpopulated_index(
            vec![
                ("pkg/clean.ts", "aaa"),
                ("pkg/deleted.ts", "bbb"),
                ("pkg/sub/clean.ts", "ccc"),
            ],
            vec![
                ("other/dirty.ts", false),
                ("pkg/deleted.ts", true),
                ("pkg/dirty.ts", false),
            ],
        );

        index.populate_untracked_from_candidates(vec![
            path("pkg/new-b.ts"),
            path("pkg/clean.ts"),
            path("pkg/dirty.ts"),
            path("pkg/deleted.ts"),
            path("pkg/new-a.ts"),
            path("other/new.ts"),
        ]);

        let (hashes, to_hash) = index.get_package_hashes(&path("pkg")).unwrap();
        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains_key(&path("clean.ts")));
        assert!(hashes.contains_key(&path("sub/clean.ts")));
        assert!(!hashes.contains_key(&path("deleted.ts")));
        assert_eq!(
            to_hash,
            vec![
                path("pkg/dirty.ts"),
                path("pkg/new-a.ts"),
                path("pkg/new-b.ts"),
            ]
        );

        index.populate_untracked_from_candidates(vec![path("pkg/new-c.ts")]);
        let (_, to_hash_after_second_populate) = index.get_package_hashes(&path("pkg")).unwrap();
        assert_eq!(to_hash_after_second_populate, to_hash);
    }
}
