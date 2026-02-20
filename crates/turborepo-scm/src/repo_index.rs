#![cfg(feature = "git2")]

use tracing::{debug, trace};
use turbopath::RelativeUnixPathBuf;

use crate::{Error, GitHashes, GitRepo, ls_tree::SortedGitHashes, status::RepoStatusEntry};

/// Pre-computed repo-wide git index that caches the results of `git ls-tree`
/// and `git status` so they can be filtered per-package without spawning
/// additional subprocesses.
///
/// Uses a sorted `Vec` for the ls-tree data so that per-package lookups can
/// use `partition_point` (binary search) for range queries. This gives the
/// same O(log n) asymptotic cost as a `BTreeMap` but with better cache
/// locality on the contiguous memory.
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
            let mut h = GitHashes::with_capacity(self.ls_tree_hashes.len());
            for (path, hash) in &self.ls_tree_hashes {
                h.insert(path.clone(), hash.clone());
            }
            h
        } else {
            // Binary search for the range of paths starting with "{prefix}/".
            // '0' is one codepoint after '/' in ASCII, so the range covers
            // exactly paths starting with the prefix followed by '/'.
            let range_start = RelativeUnixPathBuf::new(format!("{}/", prefix_str)).unwrap();
            let range_end = RelativeUnixPathBuf::new(format!("{}0", prefix_str)).unwrap();
            let lo = self
                .ls_tree_hashes
                .partition_point(|(k, _)| *k < range_start);
            let hi = self.ls_tree_hashes.partition_point(|(k, _)| *k < range_end);
            let mut h = GitHashes::new();
            for (path, hash) in &self.ls_tree_hashes[lo..hi] {
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
        let status_entries = status
            .into_iter()
            .map(|(p, is_delete)| RepoStatusEntry {
                path: path(p),
                is_delete,
            })
            .collect();
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
        assert!(hashes.contains_key(&path("apps/web/src/index.ts")));
        assert!(hashes.contains_key(&path("packages/ui/button.tsx")));
        assert!(hashes.contains_key(&path("root-file.json")));
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
        // "apps/web-admin" should NOT match when filtering for "apps/web"
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
        assert!(!hashes.contains_key(&path("deleted.ts")));
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
        // file.ts was deleted via status
        assert!(hashes.is_empty());
        // new.ts is untracked/modified
        assert_eq!(to_hash, vec![path("new.ts")]);
    }

    // Verifies that BTreeMap range queries produce correct results for
    // prefix-based package filtering. This captures the exact behavior that
    // must be preserved when switching to a sorted Vec with partition_point.
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

        // Verify the BTreeMap range query results for various prefixes.
        // These exact results must be preserved after the Vec migration.

        // "apps/web" should match apps/web/* but NOT apps/web-admin/*
        let (hashes, _) = index.get_package_hashes(&path("apps/web")).unwrap();
        assert_eq!(hashes.len(), 3);
        assert!(hashes.contains_key(&path("package.json")));
        assert!(hashes.contains_key(&path("src/index.ts")));
        assert!(hashes.contains_key(&path("src/utils.ts")));

        // "apps/docs" should match exactly 2 files
        let (hashes, _) = index.get_package_hashes(&path("apps/docs")).unwrap();
        assert_eq!(hashes.len(), 2);

        // "packages/ui" should match exactly 2 files
        let (hashes, _) = index.get_package_hashes(&path("packages/ui")).unwrap();
        assert_eq!(hashes.len(), 2);

        // A prefix that matches nothing
        let (hashes, _) = index.get_package_hashes(&path("nonexistent")).unwrap();
        assert_eq!(hashes.len(), 0);

        // Also verify via sorted Vec + binary search to confirm equivalence
        let sorted_vec: Vec<(RelativeUnixPathBuf, String)> = ls_tree_data
            .iter()
            .map(|(p, h)| (path(p), h.to_string()))
            .collect();
        // Data is already in sorted order from git ls-tree
        assert!(
            sorted_vec.windows(2).all(|w| w[0].0 < w[1].0),
            "test data must be sorted to simulate git ls-tree output"
        );

        let prefix = "apps/web";
        let range_start = path(&format!("{prefix}/"));
        let range_end = path(&format!("{prefix}0"));
        let lo = sorted_vec.partition_point(|(k, _)| *k < range_start);
        let hi = sorted_vec.partition_point(|(k, _)| *k < range_end);
        let vec_results: Vec<_> = sorted_vec[lo..hi]
            .iter()
            .map(|(p, h)| (p.clone(), h.clone()))
            .collect();

        // BTreeMap range and Vec partition_point must yield same entries
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

    // Verifies that the full-copy path (empty prefix) correctly copies all
    // entries. Important because the Vec migration changes iteration syntax.
    #[test]
    fn test_full_copy_preserves_all_entries() {
        let ls_tree_data = vec![("a.ts", "111"), ("b/c.ts", "222"), ("d/e/f.ts", "333")];
        let index = make_index(ls_tree_data, vec![]);
        let (hashes, to_hash) = index.get_package_hashes(&path("")).unwrap();
        assert_eq!(hashes.len(), 3);
        assert_eq!(hashes.get(&path("a.ts")).unwrap(), "111");
        assert_eq!(hashes.get(&path("b/c.ts")).unwrap(), "222");
        assert_eq!(hashes.get(&path("d/e/f.ts")).unwrap(), "333");
        assert!(to_hash.is_empty());
    }
}
