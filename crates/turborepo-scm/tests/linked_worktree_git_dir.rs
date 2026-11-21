//! Git exports `GIT_DIR` to hooks run inside a linked worktree and leaves
//! `GIT_WORK_TREE` unset. Turbo's per-package queries must still answer for the
//! directory they run in, so that the repository's index and ignore rules keep
//! applying to the package being hashed.
//!
//! The exported variable is process-wide, so this file holds a single test and
//! cargo gives it a test binary of its own.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turbopath::AbsoluteSystemPathBuf;
use turborepo_scm::SCM;

fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn absolute(path: &Path) -> AbsoluteSystemPathBuf {
    AbsoluteSystemPathBuf::try_from(path)
        .unwrap()
        .to_realpath()
        .unwrap()
}

/// The git directory a hook in the worktree inherits from `GIT_DIR`.
fn hook_git_dir(worktree: &Path) -> PathBuf {
    let dot_git = std::fs::read_to_string(worktree.join(".git")).unwrap();
    let gitdir = dot_git
        .strip_prefix("gitdir: ")
        .expect("linked worktree .git is a gitdir pointer")
        .trim();
    let gitdir = PathBuf::from(gitdir);
    if gitdir.is_absolute() {
        gitdir
    } else {
        worktree.join(gitdir)
    }
}

#[test]
fn package_queries_ignore_git_dir_exported_to_worktree_hooks() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let package = repo.join("packages/app");
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::write(repo.join(".gitignore"), "node_modules\n").unwrap();
    std::fs::write(package.join("package.json"), "{\"name\":\"app\"}\n").unwrap();
    std::fs::write(package.join("src/index.ts"), "export {};\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);

    let worktree = tmp.path().join("worktree");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().unwrap(),
            "-b",
            "side",
        ],
    );
    let package = absolute(&worktree.join("packages/app"));
    let dep = package.join_components(&["node_modules", "dep", "index.js"]);
    dep.ensure_dir().unwrap();
    dep.create_with_contents("module.exports = {};\n").unwrap();

    // SAFETY: this binary holds a single test, so no other thread reads the
    // environment while it is mutated.
    unsafe { std::env::set_var("GIT_DIR", hook_git_dir(&worktree)) };
    let scm = SCM::new(&package);
    let git_root = scm.git_root().map(|root| root.to_owned());
    let tracked = match &scm {
        SCM::Git(git) => Some(git.git_ls_tree(&package).unwrap()),
        SCM::Manual(_) => None,
    };
    unsafe { std::env::remove_var("GIT_DIR") };

    assert_eq!(
        git_root,
        Some(absolute(&worktree)),
        "the package belongs to the worktree's repository, not to itself"
    );
    let tracked = tracked.expect("git scm");
    let mut paths: Vec<&str> = tracked.keys().map(|path| path.as_str()).collect();
    paths.sort();
    assert_eq!(
        paths,
        ["package.json", "src/index.ts"],
        "tracked files are the package's own, not the whole repository's"
    );
}
