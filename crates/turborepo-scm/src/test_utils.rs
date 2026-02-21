//! Shared test helpers for creating temporary git repos with known state.
//!
//! These helpers are used across multiple test modules to avoid duplicating
//! the boilerplate of setting up git repos for testing.

use std::process::Command;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

pub fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
    let tmp_dir = tempfile::tempdir().unwrap();
    let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
        .unwrap()
        .to_realpath()
        .unwrap();
    (tmp_dir, dir)
}

pub fn require_git_cmd(repo_root: &AbsoluteSystemPath, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
    assert!(
        output.status.success(),
        "git {:?} failed in {}: {}",
        args,
        repo_root,
        String::from_utf8_lossy(&output.stderr),
    );
}

pub fn init_repo(repo_root: &AbsoluteSystemPath) {
    let cmds: &[&[&str]] = &[
        &["init", "."],
        &["config", "--local", "user.name", "test"],
        &["config", "--local", "user.email", "test@example.com"],
    ];
    for cmd in cmds {
        require_git_cmd(repo_root, cmd);
    }
}

pub fn commit_all(repo_root: &AbsoluteSystemPath) {
    let cmds: &[&[&str]] = &[&["add", "."], &["commit", "-m", "test commit"]];
    for cmd in cmds {
        require_git_cmd(repo_root, cmd);
    }
}
