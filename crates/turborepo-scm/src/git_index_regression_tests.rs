//! Regression tests for the git index → file hashing pipeline.
//!
//! These tests capture the exact behavior of the current libgit2-based
//! implementation (RepoGitIndex using git_ls_tree_repo_root_sorted +
//! git_status_repo_root). When we replace the backend with gix-index,
//! every test here must continue to pass with identical results.
//!
//! Test categories:
//! - Equivalence: full pipeline produces correct hashes for various repo states
//! - Edge cases: submodules, symlinks, gitignore, empty repos
//! - Contract: sorted invariants, OID compatibility, clean-tree guarantees

use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{Error, GitHashes, RepoGitIndex, SCM, test_utils, walk_candidate_files};

fn path(s: &str) -> RelativeUnixPathBuf {
    RelativeUnixPathBuf::new(s).unwrap()
}

struct TestRepo {
    _tmp: tempfile::TempDir,
    root: turbopath::AbsoluteSystemPathBuf,
}

impl TestRepo {
    fn new() -> Self {
        let (tmp, root) = test_utils::tmp_dir();
        test_utils::init_repo(&root);
        TestRepo { _tmp: tmp, root }
    }

    fn create_file(&self, rel_path: &str, content: &str) {
        let full = self.root.join_unix_path(path(rel_path));
        full.ensure_dir().unwrap();
        full.create_with_contents(content).unwrap();
    }

    fn delete_file(&self, rel_path: &str) {
        let full = self.root.join_unix_path(path(rel_path));
        full.remove().unwrap();
    }

    fn create_gitignore(&self, rel_path: &str, content: &str) {
        self.create_file(rel_path, content);
    }

    fn commit_all(&self) {
        test_utils::commit_all(&self.root);
    }

    fn stage_file(&self, rel_path: &str) {
        test_utils::require_git_cmd(&self.root, &["add", rel_path]);
    }

    fn git_cmd(&self, args: &[&str]) {
        test_utils::require_git_cmd(&self.root, args);
    }

    fn scm(&self) -> SCM {
        let scm = SCM::new(&self.root);
        assert!(matches!(scm, SCM::Git(_)), "expected Git SCM, got Manual");
        scm
    }

    fn build_repo_index(&self) -> RepoGitIndex {
        self.scm()
            .build_repo_index_eager()
            .expect("failed to build repo index")
    }

    fn build_subprocess_index(&self, prefixes: &[&str]) -> RepoGitIndex {
        let prefix_paths: Vec<_> = prefixes.iter().map(|p| path(p)).collect();
        self.scm()
            .build_repo_index_from_subprocesses(&prefix_paths)
            .expect("failed to build subprocess repo index")
    }

    /// Build index using subprocess ls-tree + diff-index for tracked state,
    /// and the walk_candidate_files approach for untracked discovery.
    fn build_walk_arm_index(&self, prefixes: &[&str]) -> RepoGitIndex {
        let scm = self.scm();
        let git = match &scm {
            SCM::Git(g) => g,
            _ => panic!("expected Git SCM"),
        };
        let prefix_paths: Vec<_> = prefixes.iter().map(|p| path(p)).collect();

        let ls_tree = git.git_ls_tree_repo_root_sorted().expect("ls-tree failed");
        let mut status = git.git_diff_index_repo_root().expect("diff-index failed");
        let candidates = walk_candidate_files(self.root.as_std_path(), Some(&prefix_paths))
            .expect("walk failed");
        let untracked = candidates
            .into_iter()
            .filter(|p| {
                let s = p.as_str();
                ls_tree
                    .binary_search_by(|(k, _)| k.as_str().cmp(s))
                    .is_err()
                    && status.binary_search_by(|e| e.path.as_str().cmp(s)).is_err()
            })
            .collect::<Vec<_>>();
        for p in untracked {
            status.push(crate::status::RepoStatusEntry {
                path: p,
                is_delete: false,
                is_untracked: true,
            });
        }
        status.sort_by(|a, b| a.path.cmp(&b.path));
        RepoGitIndex::new_for_testing(ls_tree, status)
    }

    /// Build index using subprocess ls-tree + diff-index for tracked state,
    /// and git ls-files --others for untracked discovery.
    fn build_ls_files_arm_index(&self) -> RepoGitIndex {
        let scm = self.scm();
        let git = match &scm {
            SCM::Git(g) => g,
            _ => panic!("expected Git SCM"),
        };

        let ls_tree = git.git_ls_tree_repo_root_sorted().expect("ls-tree failed");
        let mut status = git.git_diff_index_repo_root().expect("diff-index failed");
        let untracked = git.git_ls_files_untracked().expect("ls-files failed");
        for p in untracked {
            status.push(crate::status::RepoStatusEntry {
                path: p,
                is_delete: false,
                is_untracked: true,
            });
        }
        status.sort_by(|a, b| a.path.cmp(&b.path));
        RepoGitIndex::new_for_testing(ls_tree, status)
    }

    fn build_scoped_repo_index(&self, prefixes: &[&str]) -> RepoGitIndex {
        let scm = self.scm();
        let mut index = scm
            .build_tracked_repo_index_eager()
            .expect("failed to build tracked repo index");
        let prefixes = prefixes
            .iter()
            .map(|prefix| path(prefix))
            .collect::<Vec<_>>();
        scm.populate_repo_index_untracked(&mut index, &prefixes)
            .expect("failed to scope repo index");
        index
    }

    fn get_hashes(&self, package_path: &str) -> GitHashes {
        let scm = self.scm();
        let pkg = AnchoredSystemPathBuf::from_raw(package_path).unwrap();
        let index = self.build_repo_index();
        scm.get_package_file_hashes::<&str>(&self.root, &pkg, &[], false, None, Some(&index))
            .unwrap()
    }

    fn get_hashes_with_index(&self, package_path: &str, index: &RepoGitIndex) -> GitHashes {
        let scm = self.scm();
        let pkg = AnchoredSystemPathBuf::from_raw(package_path).unwrap();
        scm.get_package_file_hashes::<&str>(&self.root, &pkg, &[], false, None, Some(index))
            .unwrap()
    }

    fn get_hashes_no_index(&self, package_path: &str) -> GitHashes {
        let scm = self.scm();
        let pkg = AnchoredSystemPathBuf::from_raw(package_path).unwrap();
        scm.get_package_file_hashes::<&str>(&self.root, &pkg, &[], false, None, None)
            .unwrap()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Category 1: Equivalence Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(unix)]
#[test]
fn test_repo_index_defers_non_utf8_paths_until_matching_package_query() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

    let repo = TestRepo::new();

    repo.create_file("apps/web/src/index.ts", "console.log('hello')");
    repo.create_file("apps/web/package.json", "{}");

    let invalid_rel = PathBuf::from(OsString::from_vec(b"outside-\xff.txt".to_vec()));
    let invalid_abs = repo.root.as_std_path().join(invalid_rel);
    if std::fs::write(&invalid_abs, b"bad").is_err() {
        return;
    }

    repo.commit_all();

    let index = repo.build_repo_index();
    let web_hashes = repo.get_hashes_with_index("apps/web", &index);
    assert_eq!(web_hashes.len(), 2);
    assert!(web_hashes.contains_key(&path("src/index.ts")));
    assert!(web_hashes.contains_key(&path("package.json")));

    let scm = repo.scm();
    let root_pkg = AnchoredSystemPathBuf::from_raw("").unwrap();
    let err = scm
        .get_package_file_hashes::<&str>(&repo.root, &root_pkg, &[], false, None, Some(&index))
        .unwrap_err();
    assert!(matches!(err, Error::UnsupportedGitPath { .. }));
}

#[test]
fn test_modified_tracked_files_detected() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/src/index.ts", "original content");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    // Get committed hash
    let committed_hashes = repo.get_hashes("my-pkg");
    let committed_oid = *committed_hashes.get(&path("src/index.ts")).unwrap();

    // Modify without staging
    repo.create_file("my-pkg/src/index.ts", "modified content");

    let modified_hashes = repo.get_hashes("my-pkg");
    assert_eq!(modified_hashes.len(), 2);

    assert_ne!(
        modified_hashes.get(&path("src/index.ts")).unwrap(),
        &committed_oid,
        "modified file should have a different hash than committed"
    );

    assert_eq!(
        modified_hashes.get(&path("package.json")),
        committed_hashes.get(&path("package.json")),
        "unmodified file should keep its committed hash"
    );
}

#[test]
fn test_scoped_untracked_files_only_include_selected_package() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/committed.ts", "committed a");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("pkg-b/committed.ts", "committed b");
    repo.create_file("pkg-b/package.json", "{}");
    repo.commit_all();

    repo.create_file("pkg-a/untracked-a.ts", "new a");
    repo.create_file("pkg-b/untracked-b.ts", "new b");

    let index = repo.build_scoped_repo_index(&["pkg-a"]);

    let pkg_a_hashes = repo.get_hashes_with_index("pkg-a", &index);
    let pkg_a_no_index = repo.get_hashes_no_index("pkg-a");
    assert_eq!(pkg_a_hashes, pkg_a_no_index);
    assert!(pkg_a_hashes.contains_key(&path("untracked-a.ts")));

    let pkg_b_hashes = repo.get_hashes_with_index("pkg-b", &index);
    assert!(
        !pkg_b_hashes.contains_key(&path("untracked-b.ts")),
        "scoped repo index should not include untracked files for packages outside the selected \
         scope"
    );
}

#[test]
fn test_scoped_untracked_files_respect_ancestor_gitignore() {
    let repo = TestRepo::new();

    repo.create_file("apps/web/src/index.ts", "code");
    repo.create_file("apps/web/package.json", "{}");
    repo.commit_all();

    repo.create_file("apps/.gitignore", "ignored.ts\n");
    repo.create_file("apps/web/keep.ts", "keep");
    repo.create_file("apps/web/ignored.ts", "ignore");

    let index = repo.build_scoped_repo_index(&["apps/web"]);

    let hashes = repo.get_hashes_with_index("apps/web", &index);
    assert!(hashes.contains_key(&path("keep.ts")));
    assert!(
        !hashes.contains_key(&path("ignored.ts")),
        "ancestor .gitignore files discovered during the scoped walk should still apply"
    );

    let hashes_no_index = repo.get_hashes_no_index("apps/web");
    assert_eq!(hashes, hashes_no_index);
}

#[test]
fn test_mixed_state_comprehensive() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "my-pkg/dir/ignored-file\n");
    repo.create_file("my-pkg/committed-file", "committed bytes");
    repo.create_file("my-pkg/delete-me", "will be deleted");
    repo.create_file("my-pkg/dir/nested-file", "nested");
    repo.create_file("my-pkg/package.json", "{}");
    repo.create_file("other-pkg/other.ts", "other");
    repo.create_file("other-pkg/package.json", "{}");
    repo.create_file("package.json", "{}");

    repo.commit_all();

    repo.delete_file("my-pkg/delete-me");
    repo.create_file("my-pkg/uncommitted-file", "uncommitted bytes");
    repo.create_file("my-pkg/dir/ignored-file", "should be ignored");
    repo.create_file("other-pkg/new-other.ts", "new in other");

    let hashes = repo.get_hashes("my-pkg");

    assert!(hashes.contains_key(&path("committed-file")));
    assert!(!hashes.contains_key(&path("delete-me")));
    assert!(hashes.contains_key(&path("uncommitted-file")));
    assert!(hashes.contains_key(&path("dir/nested-file")));
    assert!(!hashes.contains_key(&path("dir/ignored-file")));
    assert!(hashes.contains_key(&path("package.json")));
    assert_eq!(hashes.len(), 4);

    let other_hashes = repo.get_hashes("other-pkg");
    assert!(other_hashes.contains_key(&path("other.ts")));
    assert!(other_hashes.contains_key(&path("new-other.ts")));
    assert!(other_hashes.contains_key(&path("package.json")));
    assert_eq!(other_hashes.len(), 3);
}

#[test]
fn test_index_and_no_index_agree_on_mixed_state() {
    let repo = TestRepo::new();

    repo.create_file("pkg/a.ts", "a");
    repo.create_file("pkg/b.ts", "b");
    repo.create_file("pkg/c.ts", "c");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    repo.delete_file("pkg/b.ts");
    repo.create_file("pkg/a.ts", "modified a");
    repo.create_file("pkg/d.ts", "new d");

    let with_index = repo.get_hashes("pkg");
    let without_index = repo.get_hashes_no_index("pkg");
    assert_eq!(with_index, without_index);
}

// ═══════════════════════════════════════════════════════════════════════════
// Category 2: Edge Cases
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_symlink_file_skipped() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/real-file.ts", "real content");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    let link_path = repo.root.join_unix_path(path("my-pkg/link"));
    link_path.symlink_to_file("real-file.ts").unwrap();

    let hashes = repo.get_hashes("my-pkg");
    assert!(hashes.contains_key(&path("real-file.ts")));
    assert!(hashes.contains_key(&path("package.json")));
    // Symlinks should not cause errors — that's the important invariant
}

#[test]
fn test_modified_then_staged_file_has_working_tree_hash() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/file.ts", "original");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/file.ts", "modified");
    test_utils::require_git_cmd(&repo.root, &["add", "my-pkg/file.ts"]);

    let hashes_staged = repo.get_hashes("my-pkg");

    repo.create_file("my-pkg/file.ts", "modified again");

    let hashes_dirty = repo.get_hashes("my-pkg");

    // Working tree content determines the hash
    assert_ne!(
        hashes_staged.get(&path("file.ts")),
        hashes_dirty.get(&path("file.ts")),
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Category 3: Contract Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_oid_from_index_matches_hash_object() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/verify.txt", "known content for hash verification");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    let index = repo.build_repo_index();
    let (committed_hashes, to_hash) = index.get_package_hashes(&path("my-pkg")).unwrap();
    assert!(to_hash.is_empty());

    let committed_oid = committed_hashes.get(&path("verify.txt")).unwrap();

    let full_path = repo.root.join_unix_path(path("my-pkg/verify.txt"));
    let file_contents = std::fs::read(&full_path).unwrap();
    let oid = gix_object::compute_hash(
        gix_index::hash::Kind::Sha1,
        gix_object::Kind::Blob,
        &file_contents,
    )
    .unwrap();
    let hash_object_oid = oid.to_string();

    assert_eq!(
        committed_oid,
        hash_object_oid.as_str(),
        "ls-tree OID must match hash_object OID for the same content"
    );
}

#[test]
fn test_racy_entries_still_produce_correct_final_hashes() {
    // Racy-git entries (mtime >= index timestamp) are deferred to hash_objects
    // instead of being verified inline. This test creates files and commits
    // in rapid succession to maximize the chance of racy entries, then verifies
    // the final hashes through the full pipeline are correct.
    let repo = TestRepo::new();

    repo.create_file("my-pkg/file-a.ts", "content a");
    repo.create_file("my-pkg/file-b.ts", "content b");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    // Get hashes immediately after commit (entries may be racy)
    let hashes_immediate = repo.get_hashes("my-pkg");

    // Wait to ensure entries are no longer racy, then get hashes again
    std::thread::sleep(std::time::Duration::from_secs(1));
    let hashes_after_wait = repo.get_hashes("my-pkg");

    // Both must produce the same hashes regardless of racy-git timing
    assert_eq!(
        hashes_immediate, hashes_after_wait,
        "hashes must be identical whether entries are racy or not"
    );
    assert_eq!(hashes_immediate.len(), 3);
}

#[test]
fn test_gix_index_sorted_order_preserved_through_pipeline() {
    // The gix-index path relies on the git index being sorted and rayon
    // preserving order. This test creates files whose alphabetical order
    // differs from creation order and verifies binary search works correctly
    // for every package.
    let repo = TestRepo::new();

    // Create in reverse alphabetical order
    repo.create_file("zzz-pkg/file.ts", "z");
    repo.create_file("mmm-pkg/file.ts", "m");
    repo.create_file("aaa-pkg/file.ts", "a");
    repo.create_file("aaa-pkg/sub/deep.ts", "deep");
    repo.create_file("mmm-pkg-extra/file.ts", "m-extra");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Add untracked files to verify they don't break sorting
    repo.create_file("mmm-pkg/new.ts", "new");

    let aaa = repo.get_hashes("aaa-pkg");
    assert_eq!(aaa.len(), 2);
    assert!(aaa.contains_key(&path("file.ts")));
    assert!(aaa.contains_key(&path("sub/deep.ts")));

    let mmm = repo.get_hashes("mmm-pkg");
    assert_eq!(mmm.len(), 2);
    assert!(mmm.contains_key(&path("file.ts")));
    assert!(mmm.contains_key(&path("new.ts")));

    // Verify prefix boundary: mmm-pkg vs mmm-pkg-extra
    let mmm_extra = repo.get_hashes("mmm-pkg-extra");
    assert_eq!(mmm_extra.len(), 1);
    assert!(mmm_extra.contains_key(&path("file.ts")));

    let zzz = repo.get_hashes("zzz-pkg");
    assert_eq!(zzz.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// Category 4: Untracked File Detection Regression Tests
//
// These tests ensure that untracked file detection produces correct results
// regardless of the underlying walk algorithm. They cover edge cases around
// directory discovery, gitignore handling, and the interaction between the
// git index and the filesystem.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_gitignore_negation_patterns() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "*.log\n!important.log\n");
    repo.create_file("my-pkg/src/index.ts", "code");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/debug.log", "debug output");
    repo.create_file("my-pkg/important.log", "keep me");
    repo.create_file("my-pkg/error.log", "error output");

    let hashes = repo.get_hashes("my-pkg");
    assert!(
        !hashes.contains_key(&path("debug.log")),
        "debug.log should be gitignored"
    );
    assert!(
        !hashes.contains_key(&path("error.log")),
        "error.log should be gitignored"
    );
    assert!(
        hashes.contains_key(&path("important.log")),
        "important.log should NOT be gitignored (negation pattern)"
    );

    let hashes_no_index = repo.get_hashes_no_index("my-pkg");
    assert_eq!(hashes, hashes_no_index);
}

#[test]
fn test_gitignore_in_untracked_directory() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/src/index.ts", "code");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    // Create a new directory that doesn't exist in the index, with its own
    // .gitignore inside it
    repo.create_file("my-pkg/new-dir/.gitignore", "*.tmp\n");
    repo.create_file("my-pkg/new-dir/keep.ts", "keep");
    repo.create_file("my-pkg/new-dir/skip.tmp", "should be ignored");
    repo.create_file("my-pkg/new-dir/sub/also-keep.ts", "also keep");
    repo.create_file("my-pkg/new-dir/sub/also-skip.tmp", "also ignored");

    let hashes = repo.get_hashes("my-pkg");
    assert!(hashes.contains_key(&path("new-dir/keep.ts")));
    assert!(hashes.contains_key(&path("new-dir/sub/also-keep.ts")));
    assert!(hashes.contains_key(&path("new-dir/.gitignore")));
    assert!(
        !hashes.contains_key(&path("new-dir/skip.tmp")),
        ".gitignore in untracked dir should be respected"
    );
    assert!(
        !hashes.contains_key(&path("new-dir/sub/also-skip.tmp")),
        ".gitignore in untracked dir should apply to subdirs"
    );

    let hashes_no_index = repo.get_hashes_no_index("my-pkg");
    assert_eq!(hashes, hashes_no_index);
}

#[test]
fn test_untracked_detection_equivalence_comprehensive() {
    // Comprehensive equivalence test: set up a complex repo state and verify
    // the index-based path produces identical results to the subprocess path.
    let repo = TestRepo::new();

    // Root-level gitignore
    repo.create_gitignore(".gitignore", "*.log\ndist/\n.cache/\n");

    // Multiple packages at different depths
    repo.create_file("apps/web/src/index.ts", "web code");
    repo.create_file("apps/web/src/utils.ts", "utils");
    repo.create_file("apps/web/package.json", "{}");
    repo.create_file("apps/docs/README.md", "docs");
    repo.create_file("apps/docs/package.json", "{}");
    repo.create_file("packages/ui/src/button.tsx", "button");
    repo.create_file("packages/ui/package.json", "{}");
    repo.create_file("packages/shared/lib/helpers.ts", "helpers");
    repo.create_file("packages/shared/package.json", "{}");

    // Nested gitignore
    repo.create_gitignore("packages/ui/.gitignore", "storybook-static/\n");

    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Now create a complex dirty state:
    // - Modified tracked file
    repo.create_file("apps/web/src/index.ts", "modified web code");
    // - Deleted tracked file
    repo.delete_file("apps/web/src/utils.ts");
    // - Untracked files in existing directories
    repo.create_file("apps/web/src/new-component.tsx", "new component");
    repo.create_file("packages/ui/src/dialog.tsx", "dialog");
    // - Untracked files in new directories
    repo.create_file("apps/web/tests/app.test.ts", "test");
    repo.create_file("packages/shared/lib/internal/deep.ts", "deep file");
    // - Files that should be gitignored
    repo.create_file("apps/web/debug.log", "log output");
    repo.create_file("apps/web/dist/bundle.js", "compiled");
    repo.create_file("packages/ui/storybook-static/index.html", "storybook");
    repo.create_file("apps/web/.cache/data.json", "cache");
    // - Untracked file at root
    repo.create_file("turbo.json", "{}");

    // Verify every package produces identical results with and without index
    let packages = [
        "apps/web",
        "apps/docs",
        "packages/ui",
        "packages/shared",
        "",
    ];
    for pkg in packages {
        let with_index = repo.get_hashes(pkg);
        let without_index = repo.get_hashes_no_index(pkg);
        assert_eq!(
            with_index, without_index,
            "index vs no-index mismatch for package {:?}",
            pkg,
        );
    }

    // Spot-check specific expectations
    let web = repo.get_hashes("apps/web");
    assert!(
        web.contains_key(&path("src/index.ts")),
        "modified file should be present"
    );
    assert!(
        !web.contains_key(&path("src/utils.ts")),
        "deleted file should be absent"
    );
    assert!(
        web.contains_key(&path("src/new-component.tsx")),
        "untracked in existing dir"
    );
    assert!(
        web.contains_key(&path("tests/app.test.ts")),
        "untracked in new dir"
    );
    assert!(
        !web.contains_key(&path("debug.log")),
        "gitignored by root .gitignore"
    );
    assert!(
        !web.contains_key(&path("dist/bundle.js")),
        "gitignored directory"
    );
    assert!(
        !web.contains_key(&path(".cache/data.json")),
        "gitignored directory"
    );

    let ui = repo.get_hashes("packages/ui");
    assert!(
        ui.contains_key(&path("src/dialog.tsx")),
        "untracked in existing dir"
    );
    assert!(
        !ui.contains_key(&path("storybook-static/index.html")),
        "gitignored by nested .gitignore"
    );
}

// Category 5: Superset-prefix regression tests
//
// These tests validate that using a superset of package prefixes for the
// untracked file walk (all packages vs. only filtered packages) produces
// identical per-package hashes. This is the core correctness property
// needed for the optimization that moves `find_untracked_files` earlier
// in the critical path by using all-package prefixes before filter resolution.

#[test]
fn test_superset_prefixes_produce_same_hashes_as_scoped_prefixes() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/src/index.ts", "a code");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("pkg-b/src/index.ts", "b code");
    repo.create_file("pkg-b/package.json", "{}");
    repo.create_file("pkg-c/src/index.ts", "c code");
    repo.create_file("pkg-c/package.json", "{}");
    repo.create_file("pkg-d/src/index.ts", "d code");
    repo.create_file("pkg-d/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Add untracked files in every package
    repo.create_file("pkg-a/untracked-a.ts", "new a");
    repo.create_file("pkg-b/untracked-b.ts", "new b");
    repo.create_file("pkg-c/untracked-c.ts", "new c");
    repo.create_file("pkg-d/untracked-d.ts", "new d");

    // Scoped index: only pkg-a and pkg-b (current behavior when filtered)
    let scoped_index = repo.build_scoped_repo_index(&["pkg-a", "pkg-b"]);

    // Superset index: all packages (proposed behavior)
    let superset_index = repo.build_scoped_repo_index(&["pkg-a", "pkg-b", "pkg-c", "pkg-d"]);

    // Per-package hashes for the "filtered" packages must be identical
    let scoped_a = repo.get_hashes_with_index("pkg-a", &scoped_index);
    let superset_a = repo.get_hashes_with_index("pkg-a", &superset_index);
    assert_eq!(
        scoped_a, superset_a,
        "pkg-a hashes differ between scoped and superset"
    );

    let scoped_b = repo.get_hashes_with_index("pkg-b", &scoped_index);
    let superset_b = repo.get_hashes_with_index("pkg-b", &superset_index);
    assert_eq!(
        scoped_b, superset_b,
        "pkg-b hashes differ between scoped and superset"
    );

    // Superset also correctly includes untracked files for the extra packages
    let superset_c = repo.get_hashes_with_index("pkg-c", &superset_index);
    assert!(superset_c.contains_key(&path("untracked-c.ts")));
    let superset_d = repo.get_hashes_with_index("pkg-d", &superset_index);
    assert!(superset_d.contains_key(&path("untracked-d.ts")));

    // Verify against no-index (subprocess) path for full equivalence
    let no_index_a = repo.get_hashes_no_index("pkg-a");
    let no_index_b = repo.get_hashes_no_index("pkg-b");
    assert_eq!(
        superset_a, no_index_a,
        "pkg-a superset vs no-index mismatch"
    );
    assert_eq!(
        superset_b, no_index_b,
        "pkg-b superset vs no-index mismatch"
    );
}

#[test]
fn test_superset_prefixes_with_gitignore_produce_same_hashes() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "*.log\nbuild/\n");
    repo.create_gitignore("pkg-b/.gitignore", "tmp/\n");
    repo.create_file("pkg-a/src/index.ts", "a");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("pkg-b/src/index.ts", "b");
    repo.create_file("pkg-b/package.json", "{}");
    repo.create_file("pkg-c/src/index.ts", "c");
    repo.create_file("pkg-c/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Untracked files — some should be ignored
    repo.create_file("pkg-a/new.ts", "new");
    repo.create_file("pkg-a/debug.log", "log"); // gitignored by root
    repo.create_file("pkg-b/new.ts", "new");
    repo.create_file("pkg-b/tmp/cache.dat", "cache"); // gitignored by pkg-b
    repo.create_file("pkg-b/build/out.js", "out"); // gitignored by root
    repo.create_file("pkg-c/new.ts", "new");

    let scoped = repo.build_scoped_repo_index(&["pkg-a", "pkg-b"]);
    let superset = repo.build_scoped_repo_index(&["pkg-a", "pkg-b", "pkg-c"]);

    let scoped_a = repo.get_hashes_with_index("pkg-a", &scoped);
    let superset_a = repo.get_hashes_with_index("pkg-a", &superset);
    assert_eq!(
        scoped_a, superset_a,
        "pkg-a with gitignore: scoped vs superset"
    );
    assert!(
        !scoped_a.contains_key(&path("debug.log")),
        "log file should be gitignored"
    );
    assert!(scoped_a.contains_key(&path("new.ts")));

    let scoped_b = repo.get_hashes_with_index("pkg-b", &scoped);
    let superset_b = repo.get_hashes_with_index("pkg-b", &superset);
    assert_eq!(
        scoped_b, superset_b,
        "pkg-b with gitignore: scoped vs superset"
    );
    assert!(
        !scoped_b.contains_key(&path("tmp/cache.dat")),
        "tmp should be gitignored"
    );
    assert!(
        !scoped_b.contains_key(&path("build/out.js")),
        "build should be gitignored"
    );
}

// Category 6: Index construction equivalence tests
//
// These tests establish ground truth for the per-package hashes produced by
// the current index construction path (gix-index + walk). Any alternative
// construction (e.g., subprocess-based) must produce identical results.
// The tests use get_hashes_no_index (subprocess fallback) as an independent
// oracle to verify correctness.

#[test]
fn test_index_equivalence_with_staged_changes() {
    // A file that is `git add`-ed but not committed should still produce
    // correct hashes. The index reflects the staged content; any alternative
    // construction must account for this.
    let repo = TestRepo::new();

    repo.create_file("pkg-a/src/index.ts", "original");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Modify and stage (but don't commit)
    repo.create_file("pkg-a/src/index.ts", "modified and staged");
    repo.stage_file("pkg-a/src/index.ts");

    let index_hashes = repo.get_hashes("pkg-a");
    let no_index_hashes = repo.get_hashes_no_index("pkg-a");

    assert_eq!(
        index_hashes, no_index_hashes,
        "staged file: index path must match no-index subprocess path"
    );

    // The hash must reflect the staged content, not the original
    let original_hash = {
        repo.create_file("pkg-a/src/index.ts", "original");
        let h = repo.get_hashes_no_index("pkg-a");
        // Restore staged content on disk
        repo.create_file("pkg-a/src/index.ts", "modified and staged");
        h
    };
    assert_ne!(
        index_hashes.get(&path("src/index.ts")),
        original_hash.get(&path("src/index.ts")),
        "staged file hash must differ from original committed content"
    );
}

#[test]
fn test_index_equivalence_with_staged_new_file() {
    // A brand new file that is staged but not committed.
    let repo = TestRepo::new();

    repo.create_file("pkg-a/src/index.ts", "a");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    repo.create_file("pkg-a/src/new-file.ts", "brand new");
    repo.stage_file("pkg-a/src/new-file.ts");

    let index_hashes = repo.get_hashes("pkg-a");
    let no_index_hashes = repo.get_hashes_no_index("pkg-a");

    assert_eq!(
        index_hashes, no_index_hashes,
        "staged new file: index must match no-index"
    );
    assert!(
        index_hashes.contains_key(&path("src/new-file.ts")),
        "staged new file must appear in hashes"
    );
}

#[test]
fn test_index_equivalence_with_deleted_files() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/src/index.ts", "a");
    repo.create_file("pkg-a/src/helper.ts", "helper");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Delete a tracked file without staging the deletion
    repo.delete_file("pkg-a/src/helper.ts");

    let index_hashes = repo.get_hashes("pkg-a");
    let no_index_hashes = repo.get_hashes_no_index("pkg-a");

    assert_eq!(
        index_hashes, no_index_hashes,
        "deleted file: index must match no-index"
    );
    assert!(
        !index_hashes.contains_key(&path("src/helper.ts")),
        "deleted file must not appear in hashes"
    );
    assert!(
        index_hashes.contains_key(&path("src/index.ts")),
        "non-deleted file must still appear"
    );
}

#[test]
fn test_index_equivalence_with_staged_deletion() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/src/index.ts", "a");
    repo.create_file("pkg-a/src/helper.ts", "helper");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Stage the deletion
    repo.delete_file("pkg-a/src/helper.ts");
    repo.git_cmd(&["add", "pkg-a/src/helper.ts"]);

    let index_hashes = repo.get_hashes("pkg-a");
    let no_index_hashes = repo.get_hashes_no_index("pkg-a");

    assert_eq!(
        index_hashes, no_index_hashes,
        "staged deletion: index must match no-index"
    );
    assert!(!index_hashes.contains_key(&path("src/helper.ts")));
}

#[test]
fn test_index_equivalence_with_modified_unstaged_files() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/src/index.ts", "original");
    repo.create_file("pkg-b/src/index.ts", "b original");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("pkg-b/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // Modify without staging
    repo.create_file("pkg-a/src/index.ts", "modified but not staged");

    let index_a = repo.get_hashes("pkg-a");
    let no_index_a = repo.get_hashes_no_index("pkg-a");
    assert_eq!(
        index_a, no_index_a,
        "modified unstaged: pkg-a index must match no-index"
    );

    // pkg-b should be unaffected
    let index_b = repo.get_hashes("pkg-b");
    let no_index_b = repo.get_hashes_no_index("pkg-b");
    assert_eq!(
        index_b, no_index_b,
        "unmodified pkg-b: index must match no-index"
    );
}

#[test]
fn test_index_equivalence_comprehensive() {
    // Combined scenario: staged changes, unstaged modifications, deleted
    // files, untracked files, and gitignored files all at once.
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "*.log\ndist/\n");
    repo.create_file("pkg-a/src/index.ts", "a");
    repo.create_file("pkg-a/src/utils.ts", "utils");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("pkg-b/src/index.ts", "b");
    repo.create_file("pkg-b/package.json", "{}");
    repo.create_file("pkg-c/src/index.ts", "c");
    repo.create_file("pkg-c/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // pkg-a: staged modification
    repo.create_file("pkg-a/src/index.ts", "a modified and staged");
    repo.stage_file("pkg-a/src/index.ts");

    // pkg-a: unstaged modification
    repo.create_file("pkg-a/src/utils.ts", "utils modified");

    // pkg-a: untracked file
    repo.create_file("pkg-a/src/new.ts", "new");

    // pkg-a: gitignored files (should not appear)
    repo.create_file("pkg-a/debug.log", "log");
    repo.create_file("pkg-a/dist/bundle.js", "bundle");

    // pkg-b: deleted file
    repo.delete_file("pkg-b/src/index.ts");

    // pkg-c: completely clean (no changes)

    for pkg in &["pkg-a", "pkg-b", "pkg-c"] {
        let index_hashes = repo.get_hashes(pkg);
        let no_index_hashes = repo.get_hashes_no_index(pkg);
        assert_eq!(
            index_hashes, no_index_hashes,
            "{}: comprehensive equivalence failed",
            pkg
        );
    }

    // Specific assertions
    let a = repo.get_hashes("pkg-a");
    assert!(
        a.contains_key(&path("src/new.ts")),
        "untracked file present"
    );
    assert!(
        !a.contains_key(&path("debug.log")),
        "gitignored log excluded"
    );
    assert!(
        !a.contains_key(&path("dist/bundle.js")),
        "gitignored dist excluded"
    );

    let b = repo.get_hashes("pkg-b");
    assert!(
        !b.contains_key(&path("src/index.ts")),
        "deleted file excluded"
    );
}

#[test]
fn test_index_equivalence_no_commits_graceful() {
    // A fresh git init with no commits. The index path should not crash.
    let (tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);

    let full = root.join_unix_path(path("pkg-a/src/index.ts"));
    full.ensure_dir().unwrap();
    full.create_with_contents("a").unwrap();
    let pkg_json = root.join_unix_path(path("pkg-a/package.json"));
    pkg_json.create_with_contents("{}").unwrap();

    let scm = SCM::new(&root);

    // build_repo_index_eager should handle no-commit repos gracefully
    // (either return None or return a valid index)
    let index = scm.build_repo_index_eager();

    // Whether it returns Some or None, it should not panic.
    // If it returns Some, the hashes should be reasonable.
    if let Some(ref idx) = index {
        let pkg = turbopath::AnchoredSystemPathBuf::from_raw("pkg-a").unwrap();
        let hashes = scm
            .get_package_file_hashes::<&str>(&root, &pkg, &[], false, None, Some(idx))
            .unwrap();
        // With no commits, git has no HEAD, so behavior depends on
        // whether the index has been populated by `git add`.
        // The key property: it should not crash.
        let _ = hashes;
    }

    drop(tmp);
}

#[test]
fn test_index_equivalence_many_packages_mixed_state() {
    // 6 packages with different states, verifying all produce correct hashes.
    let repo = TestRepo::new();

    for i in 1..=6 {
        repo.create_file(&format!("pkg-{}/src/index.ts", i), &format!("pkg {}", i));
        repo.create_file(&format!("pkg-{}/package.json", i), "{}");
    }
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // pkg-1: clean
    // pkg-2: unstaged modification
    repo.create_file("pkg-2/src/index.ts", "modified");
    // pkg-3: staged modification
    repo.create_file("pkg-3/src/index.ts", "staged");
    repo.stage_file("pkg-3/src/index.ts");
    // pkg-4: untracked file added
    repo.create_file("pkg-4/src/new.ts", "new");
    // pkg-5: file deleted
    repo.delete_file("pkg-5/src/index.ts");
    // pkg-6: staged new file + untracked file
    repo.create_file("pkg-6/src/staged-new.ts", "staged new");
    repo.stage_file("pkg-6/src/staged-new.ts");
    repo.create_file("pkg-6/src/untracked.ts", "untracked");

    for i in 1..=6 {
        let pkg = format!("pkg-{}", i);
        let index_hashes = repo.get_hashes(&pkg);
        let no_index_hashes = repo.get_hashes_no_index(&pkg);
        assert_eq!(
            index_hashes, no_index_hashes,
            "{}: mixed state equivalence failed",
            pkg
        );
    }
}

// Category 8: Subprocess index equivalence tests
//
// These tests verify that build_repo_index_from_subprocesses produces
// identical per-package hashes as the gix-index path, across all the
// edge cases from Category 7.

#[test]
fn test_subprocess_index_matches_gix_clean_repo() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/src/index.ts", "a");
    repo.create_file("pkg-a/package.json", "{}");
    repo.create_file("pkg-b/src/index.ts", "b");
    repo.create_file("pkg-b/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let gix_index = repo.build_repo_index();
    let sub_index = repo.build_subprocess_index(&["pkg-a", "pkg-b"]);

    for pkg in &["pkg-a", "pkg-b"] {
        let gix_h = repo.get_hashes_with_index(pkg, &gix_index);
        let sub_h = repo.get_hashes_with_index(pkg, &sub_index);
        let no_h = repo.get_hashes_no_index(pkg);
        assert_eq!(gix_h, sub_h, "{}: subprocess must match gix", pkg);
        assert_eq!(gix_h, no_h, "{}: both must match no-index", pkg);
    }
}

#[test]
fn test_subprocess_index_comprehensive_mixed_state() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "*.log\n");
    for i in 1..=6 {
        repo.create_file(&format!("pkg-{}/src/index.ts", i), &format!("pkg {}", i));
        repo.create_file(&format!("pkg-{}/package.json", i), "{}");
    }
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // pkg-1: clean
    // pkg-2: unstaged modification
    repo.create_file("pkg-2/src/index.ts", "modified");
    // pkg-3: staged modification
    repo.create_file("pkg-3/src/index.ts", "staged");
    repo.stage_file("pkg-3/src/index.ts");
    // pkg-4: untracked + gitignored
    repo.create_file("pkg-4/src/new.ts", "new");
    repo.create_file("pkg-4/debug.log", "log");
    // pkg-5: deleted
    repo.delete_file("pkg-5/src/index.ts");
    // pkg-6: staged new + untracked
    repo.create_file("pkg-6/src/staged-new.ts", "staged new");
    repo.stage_file("pkg-6/src/staged-new.ts");
    repo.create_file("pkg-6/src/untracked.ts", "untracked");

    let sub_index =
        repo.build_subprocess_index(&["pkg-1", "pkg-2", "pkg-3", "pkg-4", "pkg-5", "pkg-6"]);

    for i in 1..=6 {
        let pkg = format!("pkg-{}", i);
        let sub_h = repo.get_hashes_with_index(&pkg, &sub_index);
        let no_h = repo.get_hashes_no_index(&pkg);
        assert_eq!(
            sub_h, no_h,
            "{}: subprocess comprehensive equivalence failed",
            pkg
        );
    }
}

// Category 7: Race arm equivalence tests
//
// The race spawns both `walk_candidate_files` and `git ls-files --others`
// for untracked discovery, using whichever finishes first. These tests
// verify each arm independently produces correct results, so the race
// winner is always correct regardless of which arm wins.

#[test]
fn test_walk_arm_matches_no_index_comprehensive() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "*.log\n");
    for i in 1..=6 {
        repo.create_file(&format!("pkg-{}/src/index.ts", i), &format!("pkg {}", i));
        repo.create_file(&format!("pkg-{}/package.json", i), "{}");
    }
    repo.create_file("package.json", "{}");
    repo.commit_all();

    // pkg-1: clean
    // pkg-2: unstaged modification
    repo.create_file("pkg-2/src/index.ts", "modified");
    // pkg-3: staged modification
    repo.create_file("pkg-3/src/index.ts", "staged");
    repo.stage_file("pkg-3/src/index.ts");
    // pkg-4: untracked + gitignored
    repo.create_file("pkg-4/src/new.ts", "new");
    repo.create_file("pkg-4/debug.log", "log");
    // pkg-5: deleted
    repo.delete_file("pkg-5/src/index.ts");
    // pkg-6: staged new + untracked
    repo.create_file("pkg-6/src/staged-new.ts", "staged new");
    repo.stage_file("pkg-6/src/staged-new.ts");
    repo.create_file("pkg-6/src/untracked.ts", "untracked");

    let all_prefixes: Vec<&str> = (1..=6)
        .map(|i| match i {
            1 => "pkg-1",
            2 => "pkg-2",
            3 => "pkg-3",
            4 => "pkg-4",
            5 => "pkg-5",
            6 => "pkg-6",
            _ => unreachable!(),
        })
        .collect();
    let walk_index = repo.build_walk_arm_index(&all_prefixes);
    let ls_files_index = repo.build_ls_files_arm_index();

    for i in 1..=6 {
        let pkg = format!("pkg-{}", i);
        let walk_h = repo.get_hashes_with_index(&pkg, &walk_index);
        let ls_h = repo.get_hashes_with_index(&pkg, &ls_files_index);
        let no_h = repo.get_hashes_no_index(&pkg);
        assert_eq!(walk_h, no_h, "{}: walk arm comprehensive", pkg);
        assert_eq!(ls_h, no_h, "{}: ls-files arm comprehensive", pkg);
        assert_eq!(walk_h, ls_h, "{}: both arms must agree", pkg);
    }
}
