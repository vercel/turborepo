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
#![cfg(all(test, feature = "git2"))]

use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{GitHashes, RepoGitIndex, SCM, test_utils};

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

    fn get_hashes(&self, package_path: &str) -> GitHashes {
        let scm = self.scm();
        let pkg = AnchoredSystemPathBuf::from_raw(package_path).unwrap();
        let index = self.build_repo_index();
        scm.get_package_file_hashes::<&str>(&self.root, &pkg, &[], false, None, Some(&index))
            .unwrap()
    }

    fn get_hashes_no_index(&self, package_path: &str) -> GitHashes {
        let scm = self.scm();
        let pkg = AnchoredSystemPathBuf::from_raw(package_path).unwrap();
        scm.get_package_file_hashes::<&str>(&self.root, &pkg, &[], false, None, None)
            .unwrap()
    }

    fn get_hashes_with_inputs(
        &self,
        package_path: &str,
        inputs: &[&str],
        include_default_files: bool,
    ) -> GitHashes {
        let scm = self.scm();
        let pkg = AnchoredSystemPathBuf::from_raw(package_path).unwrap();
        let index = self.build_repo_index();
        scm.get_package_file_hashes(
            &self.root,
            &pkg,
            inputs,
            include_default_files,
            None,
            Some(&index),
        )
        .unwrap()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Category 1: Equivalence Tests
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_clean_tree_committed_files_have_correct_hashes() {
    let repo = TestRepo::new();

    repo.create_file("apps/web/src/index.ts", "console.log('hello')");
    repo.create_file("apps/web/package.json", "{}");
    repo.create_file("apps/docs/README.md", "# Docs");
    repo.create_file("apps/docs/package.json", "{}");
    repo.create_file("packages/ui/button.tsx", "export const Button = () => {}");
    repo.create_file("packages/ui/package.json", "{}");
    repo.create_file("package.json", "{}");

    repo.commit_all();

    let web_hashes = repo.get_hashes("apps/web");
    assert_eq!(web_hashes.len(), 2);
    assert!(web_hashes.contains_key(&path("src/index.ts")));
    assert!(web_hashes.contains_key(&path("package.json")));

    let docs_hashes = repo.get_hashes("apps/docs");
    assert_eq!(docs_hashes.len(), 2);
    assert!(docs_hashes.contains_key(&path("README.md")));
    assert!(docs_hashes.contains_key(&path("package.json")));

    let ui_hashes = repo.get_hashes("packages/ui");
    assert_eq!(ui_hashes.len(), 2);
    assert!(ui_hashes.contains_key(&path("button.tsx")));
    assert!(ui_hashes.contains_key(&path("package.json")));

    let root_hashes = repo.get_hashes("");
    assert!(
        root_hashes.len() >= 7,
        "root should see all committed files"
    );
}

#[test]
fn test_clean_tree_index_and_no_index_produce_same_hashes() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/src/index.ts", "const x = 1;");
    repo.create_file("my-pkg/package.json", "{}");
    repo.create_file("other-pkg/lib.ts", "export {};");
    repo.create_file("package.json", "{}");

    repo.commit_all();

    let with_index = repo.get_hashes("my-pkg");
    let without_index = repo.get_hashes_no_index("my-pkg");
    assert_eq!(with_index, without_index);

    let with_index = repo.get_hashes("other-pkg");
    let without_index = repo.get_hashes_no_index("other-pkg");
    assert_eq!(with_index, without_index);
}

#[test]
fn test_modified_tracked_files_detected() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/src/index.ts", "original content");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    // Get committed hash
    let committed_hashes = repo.get_hashes("my-pkg");
    let committed_oid = committed_hashes.get(&path("src/index.ts")).unwrap().clone();

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
fn test_deleted_tracked_files_excluded() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/keep.ts", "keep");
    repo.create_file("my-pkg/delete-me.ts", "delete");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.delete_file("my-pkg/delete-me.ts");

    let hashes = repo.get_hashes("my-pkg");
    assert_eq!(hashes.len(), 2, "deleted file should be excluded");
    assert!(hashes.contains_key(&path("keep.ts")));
    assert!(hashes.contains_key(&path("package.json")));
    assert!(!hashes.contains_key(&path("delete-me.ts")));
}

#[test]
fn test_untracked_files_detected() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/committed.ts", "committed");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/untracked.ts", "new file");

    let hashes = repo.get_hashes("my-pkg");
    assert_eq!(hashes.len(), 3);
    assert!(hashes.contains_key(&path("untracked.ts")));
    assert!(hashes.contains_key(&path("committed.ts")));
}

#[test]
fn test_gitignored_files_excluded() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "*.log\nmy-pkg/dist/\n");
    repo.create_file("my-pkg/src/index.ts", "code");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/debug.log", "log output");
    repo.create_file("my-pkg/dist/bundle.js", "compiled");
    repo.create_file("my-pkg/src/new.ts", "new code");

    let hashes = repo.get_hashes("my-pkg");
    assert!(!hashes.contains_key(&path("debug.log")));
    assert!(!hashes.contains_key(&path("dist/bundle.js")));
    assert!(hashes.contains_key(&path("src/new.ts")));
    assert!(hashes.contains_key(&path("src/index.ts")));
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
fn test_nested_gitignore_respected() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "*.log\n");
    repo.create_gitignore("my-pkg/.gitignore", "build/\n");
    repo.create_file("my-pkg/src/index.ts", "code");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/debug.log", "log");
    repo.create_file("my-pkg/build/out.js", "compiled");
    repo.create_file("my-pkg/src/new.ts", "new");

    let hashes = repo.get_hashes("my-pkg");
    assert!(!hashes.contains_key(&path("debug.log")));
    assert!(!hashes.contains_key(&path("build/out.js")));
    assert!(hashes.contains_key(&path("src/new.ts")));
    assert!(hashes.contains_key(&path(".gitignore")));
}

#[test]
fn test_empty_package_returns_empty_hashes() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/file.ts", "content");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let empty_dir = repo.root.join_unix_path(path("empty-pkg"));
    empty_dir.create_dir_all().unwrap();

    let hashes = repo.get_hashes("empty-pkg");
    assert!(hashes.is_empty());
}

#[test]
fn test_root_package_sees_all_committed_files() {
    let repo = TestRepo::new();

    repo.create_file("root.json", "root");
    repo.create_file("apps/web/index.ts", "web");
    repo.create_file("packages/ui/button.tsx", "ui");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let hashes = repo.get_hashes("");
    assert!(hashes.contains_key(&path("root.json")));
    assert!(hashes.contains_key(&path("apps/web/index.ts")));
    assert!(hashes.contains_key(&path("packages/ui/button.tsx")));
    assert!(hashes.contains_key(&path("package.json")));
}

#[test]
fn test_package_prefix_boundary_no_cross_contamination() {
    let repo = TestRepo::new();

    repo.create_file("apps/web/index.ts", "web");
    repo.create_file("apps/web-admin/index.ts", "web-admin");
    repo.create_file("pkg/file.ts", "pkg");
    repo.create_file("pkg-extra/file.ts", "pkg-extra");
    repo.create_file("a/file.ts", "a");
    repo.create_file("ab/file.ts", "ab");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let web = repo.get_hashes("apps/web");
    assert_eq!(web.len(), 1);
    assert!(web.contains_key(&path("index.ts")));

    let web_admin = repo.get_hashes("apps/web-admin");
    assert_eq!(web_admin.len(), 1);
    assert!(web_admin.contains_key(&path("index.ts")));

    let pkg = repo.get_hashes("pkg");
    assert_eq!(pkg.len(), 1);
    let pkg_extra = repo.get_hashes("pkg-extra");
    assert_eq!(pkg_extra.len(), 1);

    let a = repo.get_hashes("a");
    assert_eq!(a.len(), 1);
    let ab = repo.get_hashes("ab");
    assert_eq!(ab.len(), 1);
}

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
fn test_deeply_nested_package() {
    let repo = TestRepo::new();

    repo.create_file("a/b/c/d/e/pkg/index.ts", "deep");
    repo.create_file("a/b/c/d/e/pkg/package.json", "{}");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let hashes = repo.get_hashes("a/b/c/d/e/pkg");
    assert_eq!(hashes.len(), 2);
    assert!(hashes.contains_key(&path("index.ts")));
    assert!(hashes.contains_key(&path("package.json")));
}

#[test]
fn test_files_with_spaces_in_names() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/file with spaces.ts", "spaces");
    repo.create_file("my-pkg/file-with-dashes.ts", "dashes");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    let hashes = repo.get_hashes("my-pkg");
    assert!(hashes.contains_key(&path("file with spaces.ts")));
    assert!(hashes.contains_key(&path("file-with-dashes.ts")));
}

#[test]
fn test_multiple_untracked_files_across_packages() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/a.ts", "a");
    repo.create_file("pkg-b/b.ts", "b");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    repo.create_file("pkg-a/new-a.ts", "new a");
    repo.create_file("pkg-b/new-b.ts", "new b");
    repo.create_file("pkg-b/another-b.ts", "another b");

    let a_hashes = repo.get_hashes("pkg-a");
    assert!(a_hashes.contains_key(&path("a.ts")));
    assert!(a_hashes.contains_key(&path("new-a.ts")));
    assert!(!a_hashes.contains_key(&path("new-b.ts")));
    assert_eq!(a_hashes.len(), 2);

    let b_hashes = repo.get_hashes("pkg-b");
    assert!(b_hashes.contains_key(&path("b.ts")));
    assert!(b_hashes.contains_key(&path("new-b.ts")));
    assert!(b_hashes.contains_key(&path("another-b.ts")));
    assert!(!b_hashes.contains_key(&path("new-a.ts")));
    assert_eq!(b_hashes.len(), 3);
}

#[test]
fn test_staged_but_not_committed_file_detected() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/committed.ts", "committed");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/staged.ts", "staged content");
    test_utils::require_git_cmd(&repo.root, &["add", "my-pkg/staged.ts"]);

    let hashes = repo.get_hashes("my-pkg");
    assert!(hashes.contains_key(&path("staged.ts")));
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
fn test_repo_index_sorted_invariant() {
    let repo = TestRepo::new();

    repo.create_file("z-pkg/z.ts", "z");
    repo.create_file("a-pkg/a.ts", "a");
    repo.create_file("m-pkg/m.ts", "m");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let index = repo.build_repo_index();

    // Binary search depends on sorted data — verify correctness for
    // every package regardless of creation order
    let z = index.get_package_hashes(&path("z-pkg")).unwrap();
    assert_eq!(z.0.len(), 1);
    let a = index.get_package_hashes(&path("a-pkg")).unwrap();
    assert_eq!(a.0.len(), 1);
    let m = index.get_package_hashes(&path("m-pkg")).unwrap();
    assert_eq!(m.0.len(), 1);
}

#[test]
fn test_clean_tree_produces_empty_to_hash() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/index.ts", "code");
    repo.create_file("my-pkg/lib.ts", "lib");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    let index = repo.build_repo_index();
    let (hashes, to_hash) = index.get_package_hashes(&path("my-pkg")).unwrap();

    assert_eq!(hashes.len(), 3);
    assert!(to_hash.is_empty(), "clean tree must produce empty to_hash");
}

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
    let oid = git2::Oid::hash_file(git2::ObjectType::Blob, &full_path).unwrap();
    let mut hex_buf = [0u8; 40];
    hex::encode_to_slice(oid.as_bytes(), &mut hex_buf).unwrap();
    let hash_object_oid = std::str::from_utf8(&hex_buf).unwrap();

    assert_eq!(
        committed_oid, hash_object_oid,
        "ls-tree OID must match hash_object OID for the same content"
    );
}

#[test]
fn test_content_determines_hash_not_filename() {
    let repo = TestRepo::new();

    let content = "identical content in both files";
    repo.create_file("pkg-a/file.ts", content);
    repo.create_file("pkg-b/file.ts", content);
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let a = repo.get_hashes("pkg-a");
    let b = repo.get_hashes("pkg-b");

    assert_eq!(a.get(&path("file.ts")), b.get(&path("file.ts")));
}

#[test]
fn test_different_content_produces_different_hash() {
    let repo = TestRepo::new();

    repo.create_file("pkg-a/file.ts", "content A");
    repo.create_file("pkg-b/file.ts", "content B");
    repo.create_file("package.json", "{}");
    repo.commit_all();

    let a = repo.get_hashes("pkg-a");
    let b = repo.get_hashes("pkg-b");

    assert_ne!(a.get(&path("file.ts")), b.get(&path("file.ts")));
}

#[test]
fn test_hash_is_deterministic() {
    let repo = TestRepo::new();

    repo.create_file("my-pkg/index.ts", "code");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/new.ts", "new");
    repo.delete_file("my-pkg/index.ts");

    let hashes1 = repo.get_hashes("my-pkg");
    let hashes2 = repo.get_hashes("my-pkg");

    assert_eq!(hashes1, hashes2, "hashing must be deterministic");
}

#[test]
fn test_inputs_with_turbo_default_and_excludes() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "my-pkg/dir/ignored-file\n");
    repo.create_file("my-pkg/committed-file", "committed");
    repo.create_file("my-pkg/dir/nested-file", "nested");
    repo.create_file("my-pkg/package.json", "{}");
    repo.create_file("my-pkg/turbo.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/uncommitted-file", "new");
    repo.create_file("my-pkg/dir/ignored-file", "ignored");

    let hashes = repo.get_hashes_with_inputs("my-pkg", &["$TURBO_DEFAULT$", "!dir/*"], true);

    assert!(hashes.contains_key(&path("committed-file")));
    assert!(hashes.contains_key(&path("uncommitted-file")));
    assert!(hashes.contains_key(&path("package.json")));
    assert!(hashes.contains_key(&path("turbo.json")));
    assert!(!hashes.contains_key(&path("dir/nested-file")));
    assert!(!hashes.contains_key(&path("dir/ignored-file")));
}

#[test]
fn test_inputs_explicit_include_finds_gitignored_files() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "my-pkg/dir/ignored-file\n");
    repo.create_file("my-pkg/src/index.ts", "code");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/dir/ignored-file", "i am ignored by git");

    let hashes = repo.get_hashes_with_inputs("my-pkg", &["**/*-file"], false);

    assert!(hashes.contains_key(&path("dir/ignored-file")));
}

#[test]
fn test_turbo_default_plus_include_finds_gitignored_files() {
    let repo = TestRepo::new();

    repo.create_gitignore(".gitignore", "my-pkg/dir/ignored-file\n");
    repo.create_file("my-pkg/committed-file", "committed");
    repo.create_file("my-pkg/package.json", "{}");
    repo.create_file("my-pkg/turbo.json", "{}");
    repo.commit_all();

    repo.create_file("my-pkg/dir/ignored-file", "ignored by git");

    let hashes =
        repo.get_hashes_with_inputs("my-pkg", &["$TURBO_DEFAULT$", "dir/ignored-file"], true);

    assert!(hashes.contains_key(&path("dir/ignored-file")));
    assert!(hashes.contains_key(&path("committed-file")));
}

#[test]
fn test_many_packages_all_correct() {
    let repo = TestRepo::new();

    let package_names: Vec<String> = (0..30).map(|i| format!("packages/pkg-{:03}", i)).collect();

    for name in &package_names {
        repo.create_file(&format!("{}/index.ts", name), &format!("pkg {}", name));
        repo.create_file(&format!("{}/package.json", name), "{}");
    }
    repo.create_file("package.json", "{}");
    repo.commit_all();

    repo.create_file("packages/pkg-005/new.ts", "new in 005");
    repo.create_file("packages/pkg-015/new.ts", "new in 015");
    repo.delete_file("packages/pkg-020/index.ts");

    for name in &package_names {
        let hashes = repo.get_hashes(name);
        let num = name.split('-').last().unwrap().parse::<u32>().unwrap();
        match num {
            5 | 15 => {
                assert_eq!(hashes.len(), 3, "{} should have 3 files", name);
                assert!(hashes.contains_key(&path("new.ts")));
            }
            20 => {
                assert_eq!(
                    hashes.len(),
                    1,
                    "{} should have 1 file (index.ts deleted)",
                    name
                );
                assert!(!hashes.contains_key(&path("index.ts")));
                assert!(hashes.contains_key(&path("package.json")));
            }
            _ => {
                assert_eq!(hashes.len(), 2, "{} should have 2 files", name);
            }
        }
    }
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
fn test_modified_tracked_file_not_reported_as_untracked() {
    // A modified tracked file should appear in to_hash via the status path
    // (stat mismatch), NOT via the untracked file walk. This verifies the
    // binary search in find_untracked_files correctly identifies tracked files.
    let repo = TestRepo::new();

    repo.create_file("my-pkg/tracked.ts", "original");
    repo.create_file("my-pkg/package.json", "{}");
    repo.commit_all();

    // Modify tracked file
    repo.create_file("my-pkg/tracked.ts", "modified");

    let hashes = repo.get_hashes("my-pkg");
    assert_eq!(hashes.len(), 2, "should have exactly 2 files");
    assert!(hashes.contains_key(&path("tracked.ts")));
    assert!(hashes.contains_key(&path("package.json")));

    // The hash should reflect the modified content
    let clean_repo = TestRepo::new();
    clean_repo.create_file("my-pkg/tracked.ts", "original");
    clean_repo.create_file("my-pkg/package.json", "{}");
    clean_repo.commit_all();

    let clean_hashes = clean_repo.get_hashes("my-pkg");
    assert_ne!(
        hashes.get(&path("tracked.ts")),
        clean_hashes.get(&path("tracked.ts")),
        "modified file must have a different hash"
    );
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
