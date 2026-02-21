#![cfg(feature = "git2")]

use tracing::{debug, trace};
use turbopath::RelativeUnixPathBuf;

use crate::{Error, GitHashes, GitRepo, ls_tree::SortedGitHashes, status::RepoStatusEntry};

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
}

impl RepoGitIndex {
    /// Build the index using the gix-index path when available, falling back to
    /// the libgit2 ls-tree + status path otherwise.
    #[tracing::instrument(skip(git))]
    pub fn new(git: &GitRepo) -> Result<Self, Error> {
        #[cfg(feature = "gix")]
        {
            match Self::new_from_gix_index(git) {
                Ok(index) => return Ok(index),
                Err(e) => {
                    debug!("gix-index path failed: {}. Falling back to libgit2.", e);
                }
            }
        }

        Self::new_from_libgit2(git)
    }

    /// Build the index by running `git ls-tree` and `git status` via libgit2
    /// on separate threads.
    fn new_from_libgit2(git: &GitRepo) -> Result<Self, Error> {
        let (ls_tree_hashes, status_entries) = std::thread::scope(|s| {
            let ls_tree = s.spawn(|| git.git_ls_tree_repo_root_sorted());
            let status = s.spawn(|| git.git_status_repo_root());
            (
                ls_tree.join().expect("ls-tree thread panicked"),
                status.join().expect("status thread panicked"),
            )
        });
        let ls_tree_hashes = ls_tree_hashes?;
        let mut status_entries = status_entries?;

        status_entries.sort_by(|a, b| a.path.cmp(&b.path));

        debug!(
            "built repo git index (libgit2): ls_tree_count={}, status_count={}",
            ls_tree_hashes.len(),
            status_entries.len(),
        );
        Ok(Self {
            ls_tree_hashes,
            status_entries,
        })
    }

    /// Build the index by reading `.git/index` directly via gix-index.
    ///
    /// This replaces both `git ls-tree` and `git status` with a single
    /// operation: reading the index file gives us committed blob OIDs, and
    /// stat-comparing each entry against the filesystem tells us which files
    /// are modified or deleted. Untracked files are detected by a parallel
    /// walk of the working tree respecting .gitignore.
    ///
    /// Racy-git entries (where mtime >= index timestamp, so we can't trust
    /// the stat comparison) are deferred to per-package hashing rather than
    /// content-hashed inline. This avoids reading every file from disk on
    /// freshly cloned/checked-out repos.
    #[cfg(feature = "gix")]
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
        let num_entries = index
            .entries()
            .iter()
            .filter(|e| !e.mode.is_submodule())
            .count();

        // Classify entries in parallel: stat each file, compare with index,
        // and carry the raw ObjectId (20 bytes, Copy) instead of a hex String.
        // The hex conversion only happens for entries classified as Clean.
        let classified: Vec<Result<EntryClassification, Error>> = index
            .entries()
            .par_iter()
            .filter(|e| !e.mode.is_submodule())
            .map(|e| {
                let path_bytes = e.path(&index);
                let path_str = std::str::from_utf8(path_bytes).map_err(|err| {
                    Error::git_error(format!("invalid utf8 in index path: {}", err))
                })?;
                let rel_path = RelativeUnixPathBuf::new(path_str)?;
                let abs_path = git.root.join_unix_path(&rel_path);

                match gix_index::fs::Metadata::from_path_no_follow(abs_path.as_std_path()) {
                    Ok(fs_meta) => {
                        let fs_stat = gix_index::entry::Stat::from_fs(&fs_meta).map_err(|err| {
                            Error::git_error(format!(
                                "failed to convert stat for {}: {}",
                                path_str, err
                            ))
                        })?;

                        let stat_matches = e.stat.matches(&fs_stat, stat_opts);

                        if !stat_matches {
                            return Ok(EntryClassification::Modified { path: rel_path });
                        }

                        let is_racy = e.stat.is_racy(index_timestamp, stat_opts);
                        if is_racy {
                            // Stat matches but mtime >= index timestamp so we
                            // can't trust it. Defer to per-package hash_objects
                            // instead of reading the file here — avoids
                            // reading every file from disk on a fresh checkout.
                            return Ok(EntryClassification::Modified { path: rel_path });
                        }

                        // Clean: stat matches and not racy — use index OID.
                        // Convert the raw ObjectId to hex only for this path.
                        Ok(EntryClassification::Clean {
                            path: rel_path,
                            oid: e.id.to_hex().to_string(),
                        })
                    }
                    Err(_) => Ok(EntryClassification::Deleted { path: rel_path }),
                }
            })
            .collect();

        let mut ls_tree_hashes = SortedGitHashes::with_capacity(num_entries);
        let mut status_entries = Vec::new();

        for result in classified {
            match result? {
                EntryClassification::Clean { path, oid } => {
                    ls_tree_hashes.push((path, oid));
                }
                EntryClassification::Modified { path } => {
                    status_entries.push(RepoStatusEntry {
                        path,
                        is_delete: false,
                    });
                }
                EntryClassification::Deleted { path } => {
                    status_entries.push(RepoStatusEntry {
                        path,
                        is_delete: true,
                    });
                }
            }
        }

        // ls_tree_hashes is already sorted (git index is sorted, rayon
        // preserves order for indexed iterators, sequential loop preserves
        // order). status_entries needs sorting after untracked files are added.

        // Detect untracked files via a parallel walk of the working tree.
        // Use binary search on the sorted ls_tree_hashes and status_entries
        // instead of building a HashSet.
        let untracked = find_untracked_files(git, &ls_tree_hashes, &status_entries)?;
        for path in untracked {
            status_entries.push(RepoStatusEntry {
                path,
                is_delete: false,
            });
        }

        status_entries.sort_by(|a, b| a.path.cmp(&b.path));

        debug!(
            "built repo git index (gix-index): clean_count={}, status_count={}",
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
            let mut h = GitHashes::with_capacity(self.ls_tree_hashes.len());
            for (path, hash) in &self.ls_tree_hashes {
                h.insert(path.clone(), hash.clone());
            }
            h
        } else {
            let range_start = RelativeUnixPathBuf::new(format!("{}/", prefix_str)).unwrap();
            let range_end = RelativeUnixPathBuf::new(format!("{}0", prefix_str)).unwrap();
            let lo = self
                .ls_tree_hashes
                .partition_point(|(k, _)| *k < range_start);
            let hi = self.ls_tree_hashes.partition_point(|(k, _)| *k < range_end);
            let mut h = GitHashes::with_capacity(hi - lo);
            for (path, hash) in &self.ls_tree_hashes[lo..hi] {
                if let Ok(stripped) = path.strip_prefix(pkg_prefix) {
                    h.insert(stripped, hash.clone());
                }
            }
            h
        };

        let mut to_hash = Vec::new();
        let status_entries = if prefix_is_empty {
            &self.status_entries[..]
        } else {
            let range_start = RelativeUnixPathBuf::new(format!("{}/", prefix_str)).unwrap();
            let range_end = RelativeUnixPathBuf::new(format!("{}0", prefix_str)).unwrap();
            let lo = self
                .status_entries
                .partition_point(|e| e.path < range_start);
            let hi = self.status_entries.partition_point(|e| e.path < range_end);
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
}

/// Walk the working tree to find untracked files (files on disk that are
/// not in the git index). Uses the `ignore` crate's parallel walker to
/// respect .gitignore rules. Uses binary search on the sorted ls_tree_hashes
/// and status_entries instead of building a HashSet.
#[cfg(feature = "gix")]
#[tracing::instrument(skip(git, ls_tree_hashes, status_entries))]
fn find_untracked_files(
    git: &GitRepo,
    ls_tree_hashes: &SortedGitHashes,
    status_entries: &[RepoStatusEntry],
) -> Result<Vec<RelativeUnixPathBuf>, Error> {
    use std::sync::Mutex;

    use ignore::WalkBuilder;

    // Pre-sort status_entries for binary search (they may not be fully sorted
    // yet if this is called before the final sort).
    let mut sorted_status: Vec<&str> = status_entries.iter().map(|e| e.path.as_str()).collect();
    sorted_status.sort_unstable();

    let untracked = Mutex::new(Vec::new());
    let root = git.root.as_std_path();

    let walker = WalkBuilder::new(root)
        .follow_links(false)
        .git_ignore(true)
        .require_git(true)
        .hidden(false)
        .filter_entry(|entry| {
            // Never descend into .git/ — the ignore crate may walk it when
            // hidden(false) is set because .git is a hidden directory.
            !(entry.file_type().is_some_and(|ft| ft.is_dir()) && entry.file_name() == ".git")
        })
        .threads(rayon::current_num_threads().min(8))
        .build_parallel();

    walker.run(|| {
        Box::new(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return ignore::WalkState::Continue,
            };

            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                return ignore::WalkState::Continue;
            }
            if entry.file_type().is_some_and(|ft| ft.is_symlink()) {
                return ignore::WalkState::Continue;
            }

            let abs_path = entry.into_path();
            let rel_path = match abs_path.strip_prefix(root) {
                Ok(rel) => rel,
                Err(_) => return ignore::WalkState::Continue,
            };

            let unix_str = match rel_path.to_str() {
                Some(s) => s,
                None => return ignore::WalkState::Continue,
            };

            #[cfg(windows)]
            let unix_str_owned = unix_str.replace('\\', "/");
            #[cfg(windows)]
            let unix_str: &str = &unix_str_owned;

            // Binary search on sorted ls_tree_hashes (O(log n), zero extra memory)
            let in_ls_tree = ls_tree_hashes
                .binary_search_by(|(p, _)| p.as_str().cmp(unix_str))
                .is_ok();
            let in_status = sorted_status.binary_search(&unix_str).is_ok();

            if !in_ls_tree
                && !in_status
                && let Ok(path) = RelativeUnixPathBuf::new(unix_str)
            {
                untracked.lock().unwrap().push(path);
            }

            ignore::WalkState::Continue
        })
    });

    Ok(untracked.into_inner().unwrap())
}

#[cfg(feature = "gix")]
enum EntryClassification {
    Clean {
        path: RelativeUnixPathBuf,
        oid: String,
    },
    Modified {
        path: RelativeUnixPathBuf,
    },
    Deleted {
        path: RelativeUnixPathBuf,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use turbopath::RelativeUnixPathBuf;

    use super::*;

    fn path(s: &str) -> RelativeUnixPathBuf {
        RelativeUnixPathBuf::new(s).unwrap()
    }

    fn make_index(ls_tree: Vec<(&str, &str)>, status: Vec<(&str, bool)>) -> RepoGitIndex {
        let mut ls_tree_hashes: SortedGitHashes = ls_tree
            .into_iter()
            .map(|(p, h)| (path(p), h.to_string()))
            .collect::<Vec<_>>();
        ls_tree_hashes.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut status_entries: Vec<RepoStatusEntry> = status
            .into_iter()
            .map(|(p, is_delete)| RepoStatusEntry {
                path: path(p),
                is_delete,
            })
            .collect();
        status_entries.sort_by(|a, b| a.path.cmp(&b.path));
        RepoGitIndex {
            ls_tree_hashes,
            status_entries,
        }
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
        assert_eq!(hashes.get(&path("src/index.ts")).unwrap(), "aaa");
        assert_eq!(hashes.get(&path("package.json")).unwrap(), "bbb");
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
            ("packages/ui/button.tsx", "ggg"),
            ("packages/ui/package.json", "hhh"),
            ("root.json", "iii"),
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
        assert_eq!(hashes.get(&path("a.ts")).unwrap(), "111");
        assert!(to_hash.is_empty());
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
}
