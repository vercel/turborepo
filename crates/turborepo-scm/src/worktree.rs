//! Git worktree detection for cache sharing between linked worktrees.
//!
//! This module provides functionality to detect whether the current repository
//! is a Git worktree and, if so, locate the main worktree's root directory.
//! This enables linked worktrees to share the local cache with the main
//! worktree.

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::Error;

/// Information about the Git worktree configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The root of the current worktree
    pub worktree_root: AbsoluteSystemPathBuf,
    /// The root of the main worktree
    pub main_worktree_root: AbsoluteSystemPathBuf,
    /// The root of the git repository (resolved from `--show-cdup`).
    /// Captured here to avoid a redundant subprocess in `SCM::new`.
    pub git_root: AbsoluteSystemPathBuf,
}

impl WorktreeInfo {
    /// Returns true if running in a linked worktree (not the main worktree).
    pub fn is_linked_worktree(&self) -> bool {
        self.worktree_root != self.main_worktree_root
    }

    /// Detect worktree configuration from a path within a Git repository.
    ///
    /// Walks up from `path` looking for a `.git` entry (directory or file),
    /// then reads the git metadata directly from the filesystem instead of
    /// spawning a `git rev-parse` subprocess.
    #[tracing::instrument]
    pub fn detect(path: &AbsoluteSystemPath) -> Result<Self, Error> {
        // Walk up from `path` to find the directory containing `.git`.
        let worktree_root = find_git_ancestor(path)?;
        let dot_git = worktree_root.join_component(".git");
        let dot_git_meta = std::fs::symlink_metadata(dot_git.as_std_path())
            .map_err(|e| Error::git_error(format!("failed to stat .git: {}", e)))?;

        if dot_git_meta.is_dir() {
            // Main worktree: .git is a directory.
            Ok(Self {
                worktree_root: worktree_root.clone(),
                main_worktree_root: worktree_root.clone(),
                git_root: worktree_root,
            })
        } else if dot_git_meta.is_file() {
            // Linked worktree: .git is a file containing "gitdir: <path>".
            let content = std::fs::read_to_string(dot_git.as_std_path())
                .map_err(|e| Error::git_error(format!("failed to read .git file: {}", e)))?;
            let gitdir_path = content
                .strip_prefix("gitdir: ")
                .ok_or_else(|| {
                    Error::git_error(format!(
                        ".git file has unexpected format (expected 'gitdir: ...'): {:?}",
                        content.trim()
                    ))
                })?
                .trim();

            // Resolve the gitdir path (may be relative to the worktree root)
            let gitdir = if std::path::Path::new(gitdir_path).is_absolute() {
                AbsoluteSystemPathBuf::try_from(gitdir_path)?
            } else {
                let resolved = worktree_root.as_std_path().join(gitdir_path);
                AbsoluteSystemPathBuf::try_from(resolved.as_path())?.to_realpath()?
            };

            // Read commondir to find the main .git directory.
            // commondir is relative to the gitdir (e.g., "../.." points from
            // .git/worktrees/<name> back to .git).
            let commondir_file = gitdir.join_component("commondir");
            let commondir_content = std::fs::read_to_string(commondir_file.as_std_path())
                .map_err(|e| Error::git_error(format!("failed to read commondir: {}", e)))?;
            let commondir_rel = commondir_content.trim();

            let git_common_path = if std::path::Path::new(commondir_rel).is_absolute() {
                AbsoluteSystemPathBuf::try_from(commondir_rel)?
            } else {
                let resolved = gitdir.as_std_path().join(commondir_rel);
                AbsoluteSystemPathBuf::try_from(resolved.as_path())?.to_realpath()?
            };

            // The main worktree root is the parent of the common .git directory.
            let main_worktree_root = git_common_path
                .parent()
                .map(|p| p.to_owned())
                .ok_or_else(|| Error::git_error("git common dir has no parent directory"))?;

            Ok(Self {
                worktree_root: worktree_root.clone(),
                main_worktree_root,
                git_root: worktree_root,
            })
        } else {
            Err(Error::git_error(
                ".git exists but is neither a directory nor a file",
            ))
        }
    }
}

/// Walk up from `path` looking for a directory that contains a `.git` entry.
fn find_git_ancestor(path: &AbsoluteSystemPath) -> Result<AbsoluteSystemPathBuf, Error> {
    let mut current = path.to_owned();
    loop {
        let dot_git = current.join_component(".git");
        if dot_git.as_std_path().exists() {
            return Ok(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_owned(),
            None => {
                return Err(Error::git_error(format!(
                    "not a git repository (or any parent up to root): {}",
                    path
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    /// Derive the main worktree root from the git common directory path.
    /// Only used by the subprocess-based test helper for equivalence testing.
    fn resolve_main_worktree_root(
        cwd: &AbsoluteSystemPath,
        git_common_dir: &str,
    ) -> Result<AbsoluteSystemPathBuf, Error> {
        let git_common_path = if std::path::Path::new(git_common_dir).is_absolute() {
            AbsoluteSystemPathBuf::try_from(git_common_dir).unwrap()
        } else {
            let resolved = cwd.as_std_path().join(git_common_dir);
            AbsoluteSystemPathBuf::try_from(resolved.as_path())
                .unwrap()
                .to_realpath()
                .unwrap()
        };

        git_common_path
            .parent()
            .map(|p| p.to_owned())
            .ok_or_else(|| Error::git_error("git common dir has no parent directory"))
    }

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
    }

    fn require_git_cmd(cwd: &AbsoluteSystemPath, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(cwd);
        let output = cmd.output().unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn setup_repository(repo_root: &AbsoluteSystemPath) {
        let cmds: &[&[&str]] = &[
            &["init", "."],
            &["config", "--local", "user.name", "test"],
            &["config", "--local", "user.email", "test@example.com"],
        ];
        for cmd in cmds {
            require_git_cmd(repo_root, cmd);
        }
        // Create an initial commit so we can create worktrees
        repo_root
            .join_component("README.md")
            .create_with_contents("# Test")
            .unwrap();
        require_git_cmd(repo_root, &["add", "."]);
        require_git_cmd(repo_root, &["commit", "-m", "Initial commit"]);
    }

    #[test]
    fn test_main_worktree_detection() {
        let (_tmp, repo_root) = tmp_dir();
        setup_repository(&repo_root);

        let info = WorktreeInfo::detect(&repo_root).unwrap();

        assert_eq!(info.worktree_root, repo_root);
        assert_eq!(info.main_worktree_root, repo_root);
        assert_eq!(info.git_root, repo_root);
        assert!(!info.is_linked_worktree());
    }

    #[test]
    fn test_linked_worktree_detection() {
        let (_tmp, repo_root) = tmp_dir();
        setup_repository(&repo_root);

        // Create a linked worktree inside our temp directory
        let worktree_path = repo_root.join_component("worktree-linked");
        require_git_cmd(
            &repo_root,
            &[
                "worktree",
                "add",
                worktree_path.as_str(),
                "-b",
                "test-branch-detection",
            ],
        );

        let info = WorktreeInfo::detect(&worktree_path).unwrap();

        assert_eq!(info.worktree_root, worktree_path);
        assert_eq!(info.main_worktree_root, repo_root);
        assert_eq!(info.git_root, worktree_path);
        assert!(info.is_linked_worktree());
    }

    #[test]
    fn test_non_git_directory() {
        let (_tmp, dir) = tmp_dir();
        // Don't initialize git - just use the temp directory

        let result = WorktreeInfo::detect(&dir);
        assert!(result.is_err());
    }

    #[test]
    fn test_subdirectory_detection() {
        let (_tmp, repo_root) = tmp_dir();
        setup_repository(&repo_root);

        // Create a subdirectory
        let subdir = repo_root.join_component("packages").join_component("app");
        subdir.create_dir_all().unwrap();

        let info = WorktreeInfo::detect(&subdir).unwrap();

        assert_eq!(info.worktree_root, repo_root);
        assert_eq!(info.main_worktree_root, repo_root);
        // git_root should resolve to repo_root even when called from subdir
        assert_eq!(info.git_root, repo_root);
        assert!(!info.is_linked_worktree());
    }

    #[test]
    fn test_linked_worktree_outside_main_repo() {
        // Reproduces the real-world scenario where the worktree lives at a sibling
        // path (e.g. ~/Vercel/front-worktree/branch) rather than inside the main
        // repo (~/Vercel/front). git_root must resolve to the worktree root so
        // that path anchoring works correctly in the SCM layer.
        let (_tmp_main, repo_root) = tmp_dir();
        let (_tmp_wt, worktree_parent) = tmp_dir();
        setup_repository(&repo_root);

        let worktree_path = worktree_parent.join_component("my-branch");
        require_git_cmd(
            &repo_root,
            &[
                "worktree",
                "add",
                worktree_path.as_str(),
                "-b",
                "test-branch-outside",
            ],
        );

        let info = WorktreeInfo::detect(&worktree_path).unwrap();

        assert_eq!(info.worktree_root, worktree_path);
        assert_eq!(info.main_worktree_root, repo_root);
        // git_root must be the worktree root, NOT the main worktree root,
        // otherwise path anchoring fails with "is not parent of"
        assert_eq!(info.git_root, worktree_path);
        assert!(info.is_linked_worktree());
    }

    #[test]
    fn test_linked_worktree_subdirectory_detection() {
        let (_tmp, repo_root) = tmp_dir();
        setup_repository(&repo_root);

        // Create a linked worktree inside our temp directory
        let worktree_path = repo_root.join_component("worktree-subdir-test");
        require_git_cmd(
            &repo_root,
            &[
                "worktree",
                "add",
                worktree_path.as_str(),
                "-b",
                "test-branch-subdir",
            ],
        );

        // Create a subdirectory in the linked worktree
        let subdir = worktree_path.join_component("packages");
        subdir.create_dir_all().unwrap();

        let info = WorktreeInfo::detect(&subdir).unwrap();

        assert_eq!(info.worktree_root, worktree_path);
        assert_eq!(info.main_worktree_root, repo_root);
        assert!(info.is_linked_worktree());
    }

    /// Detect worktree info using a git subprocess (the old implementation).
    /// Used as a reference to verify the pure-Rust implementation.
    fn detect_via_subprocess(
        path: &AbsoluteSystemPath,
    ) -> Result<WorktreeInfo, Box<dyn std::error::Error>> {
        let output = Command::new("git")
            .args([
                "rev-parse",
                "--show-toplevel",
                "--git-common-dir",
                "--show-cdup",
            ])
            .current_dir(path)
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "git rev-parse failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut lines = stdout.lines();

        let toplevel = lines.next().unwrap().trim();
        let worktree_root = AbsoluteSystemPathBuf::try_from(toplevel)?;

        let git_common_dir = lines.next().unwrap().trim().to_string();

        let show_cdup = lines.next().unwrap().trim();
        let git_root = if show_cdup.is_empty() {
            path.to_owned()
        } else {
            let resolved = path.as_std_path().join(show_cdup);
            AbsoluteSystemPathBuf::try_from(resolved.as_path())?.to_realpath()?
        };

        let main_worktree_root = resolve_main_worktree_root(path, &git_common_dir)?;

        Ok(WorktreeInfo {
            worktree_root,
            main_worktree_root,
            git_root,
        })
    }

    #[test]
    fn test_equivalence_with_subprocess() {
        let (_tmp, repo_root) = tmp_dir();
        setup_repository(&repo_root);

        // Create a linked worktree
        let worktree_path = repo_root.join_component("worktree-equiv");
        require_git_cmd(
            &repo_root,
            &[
                "worktree",
                "add",
                worktree_path.as_str(),
                "-b",
                "test-equiv",
            ],
        );

        let subdir = repo_root.join_component("packages").join_component("app");
        subdir.create_dir_all().unwrap();

        let wt_subdir = worktree_path.join_component("src");
        wt_subdir.create_dir_all().unwrap();

        // Verify both implementations agree for every scenario
        let test_paths = [
            ("main root", &repo_root),
            ("main subdir", &subdir),
            ("worktree root", &worktree_path),
            ("worktree subdir", &wt_subdir),
        ];

        for (label, path) in &test_paths {
            let pure_rust = WorktreeInfo::detect(path).unwrap();
            let subprocess = detect_via_subprocess(path).unwrap();
            assert_eq!(
                pure_rust.worktree_root, subprocess.worktree_root,
                "{label}: worktree_root mismatch"
            );
            assert_eq!(
                pure_rust.main_worktree_root, subprocess.main_worktree_root,
                "{label}: main_worktree_root mismatch"
            );
            assert_eq!(
                pure_rust.git_root, subprocess.git_root,
                "{label}: git_root mismatch"
            );
        }
    }
}
