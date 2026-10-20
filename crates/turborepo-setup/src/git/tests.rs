//! Contract tests: the real `git` executable against real repositories. These
//! are the only setup unit tests that spawn Git; everything else uses
//! [`FakeGit`], so keep this set small and keep both sides answering alike.

use std::{fs, process::Command};

use super::*;

fn git(dir: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(arguments)
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

#[test]
fn broken_git_marker_exits_128() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".git"), "").unwrap();
    let fake = FakeGit::new();
    fake.fail(Some(Failure::Fatal("fatal: invalid gitfile format".into())));
    for git in [&GitCli as &dyn Git, &fake] {
        let reply = git.toplevel(root.path()).unwrap();
        assert_eq!(reply.code, Some(128));
        assert!(!reply.stderr.is_empty());
    }
}

#[test]
fn separate_git_dir_reports_the_worktree_toplevel() {
    let base = tempfile::tempdir().unwrap();
    let work = base.path().join("work");
    git(
        base.path(),
        &[
            "init",
            "--quiet",
            "--separate-git-dir",
            "gitdir",
            work.to_str().unwrap(),
        ],
    );
    assert!(work.join(".git").is_file());
    let nested = work.join("apps/web");
    fs::create_dir_all(&nested).unwrap();
    let fake = FakeGit::new();
    fake.set_toplevel(&nested, &work.canonicalize().unwrap());
    for git in [&GitCli as &dyn Git, &fake] {
        let reply = git.toplevel(&nested).unwrap();
        assert!(reply.success());
        let toplevel = std::str::from_utf8(&reply.stdout).unwrap().trim();
        assert_eq!(
            Path::new(toplevel).canonicalize().unwrap(),
            work.canonicalize().unwrap()
        );
    }
}

#[test]
fn missing_git_executable_is_explained() {
    let error = git_spawn_error(io::ErrorKind::NotFound.into());
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(error.to_string().contains("not found on PATH"));
    let other = git_spawn_error(io::Error::other("other"));
    assert_eq!(other.to_string(), "other");
    let fake = FakeGit::new();
    fake.fail(Some(Failure::Spawn));
    let error = fake.toplevel(Path::new(".")).unwrap_err();
    assert!(error.to_string().contains("not found on PATH"));
    assert_eq!(fake.queries(), 1);
}
