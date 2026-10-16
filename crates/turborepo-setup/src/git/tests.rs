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

fn repository() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    root
}

const STORAGE: [&str; 2] = [".turbo/setup-lock/writer", ".turbo/setup-lock/staged"];

#[test]
fn check_ignore_reports_exactly_the_ignored_paths() {
    let root = repository();
    let fake = FakeGit::new();
    for git in [&GitCli as &dyn Git, &fake] {
        let reply = git.check_ignore(root.path(), &[".turbo/"]).unwrap();
        assert_eq!((reply.code, reply.stdout), (Some(1), Vec::new()));
    }
    fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
    for git in [&GitCli as &dyn Git, &fake] {
        let reply = git.check_ignore(root.path(), &[".turbo/"]).unwrap();
        assert!(reply.success());
        assert_eq!(reply.stdout, b".turbo/\n");
        let reply = git.check_ignore(root.path(), &STORAGE).unwrap();
        assert!(reply.success());
        assert_eq!(
            reply.stdout,
            format!("{}\n{}\n", STORAGE[0], STORAGE[1]).as_bytes()
        );
    }
    // A re-included path is not ignored, so the pair no longer matches.
    fs::write(
        root.path().join(".gitignore"),
        "/.turbo/*\n!/.turbo/setup-lock/\n",
    )
    .unwrap();
    let reply = GitCli.check_ignore(root.path(), &STORAGE).unwrap();
    assert_ne!(
        reply.stdout,
        format!("{}\n{}\n", STORAGE[0], STORAGE[1]).as_bytes()
    );
}

#[test]
fn tracked_storage_is_listed_and_not_reported_as_ignored() {
    let root = repository();
    fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
    fs::create_dir_all(root.path().join(".turbo/setup-lock")).unwrap();
    fs::write(root.path().join(STORAGE[0]), "tracked").unwrap();
    git(root.path(), &["add", "--force", "--", STORAGE[0]]);
    let fake = FakeGit::new();
    fake.track(STORAGE[0]);
    for git in [&GitCli as &dyn Git, &fake] {
        let tracked = git.ls_files(root.path(), &[".turbo/"]).unwrap();
        assert!(tracked.success());
        assert_eq!(tracked.stdout, format!("{}\n", STORAGE[0]).as_bytes());
        let ignored = git.check_ignore(root.path(), &STORAGE).unwrap();
        assert_eq!(ignored.stdout, format!("{}\n", STORAGE[1]).as_bytes());
        // A directory holding a tracked file is not reported either.
        let ignored = git.check_ignore(root.path(), &[".turbo/"]).unwrap();
        assert_eq!((ignored.code, ignored.stdout), (Some(1), Vec::new()));
    }
}

#[test]
fn broken_git_marker_exits_128() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(".git"), "").unwrap();
    fs::write(root.path().join(".gitignore"), "/.turbo/\n").unwrap();
    let fake = FakeGit::new();
    fake.fail(Some(Failure::Fatal("fatal: invalid gitfile format".into())));
    for git in [&GitCli as &dyn Git, &fake] {
        let reply = git.check_ignore(root.path(), &[".turbo/"]).unwrap();
        assert_eq!(reply.code, Some(128));
        assert!(!reply.stderr.is_empty());
        assert!(git.toplevel(root.path()).unwrap().code == Some(128));
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
    let error = fake.check_ignore(Path::new("."), &[".turbo/"]).unwrap_err();
    assert!(error.to_string().contains("not found on PATH"));
    assert_eq!(fake.queries(), 1);
}
