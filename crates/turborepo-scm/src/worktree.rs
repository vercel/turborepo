//! Git worktree detection for cache sharing between linked worktrees.
//!
//! This module provides functionality to detect whether the current repository
//! is a Git worktree and, if so, locate the main worktree's root directory.
//! This enables linked worktrees to share the local cache with the main
//! worktree.

use std::process::Command;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::Error;

/// Information about the Git worktree configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The root of the current worktree
    pub worktree_root: AbsoluteSystemPathBuf,
    /// The root of the main worktree
    pub main_worktree_root: AbsoluteSystemPathBuf,
}

impl WorktreeInfo {
    /// Returns true if running in a linked worktree (not the main worktree).
    pub fn is_linked_worktree(&self) -> bool {
        self.worktree_root != self.main_worktree_root
    }

    /// Detect worktree configuration from a path within a Git repository.
    ///
    /// Uses Git commands to determine:
    /// - The current worktree root (`git rev-parse --show-toplevel`)
    /// - The shared git directory (`git rev-parse --git-common-dir`)
    /// - The main worktree root (derived from the git common directory)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The path is not within a Git repository
    /// - Git commands fail to execute
    /// - The worktree structure cannot be determined
    #[tracing::instrument]
    pub fn detect(path: &AbsoluteSystemPath) -> Result<Self, Error> {
        let worktree_root = get_worktree_root(path)?;
        let main_worktree_root = get_main_worktree_root(path)?;

        Ok(Self {
            worktree_root,
            main_worktree_root,
        })
    }
}

/// Get the root of the current worktree using `git rev-parse --show-toplevel`.
fn get_worktree_root(path: &AbsoluteSystemPath) -> Result<AbsoluteSystemPathBuf, Error> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::git_error(format!(
            "git rev-parse --show-toplevel failed: {stderr}"
        )));
    }

    let toplevel = String::from_utf8(output.stdout)?.trim().to_string();
    AbsoluteSystemPathBuf::try_from(toplevel.as_str()).map_err(|e| e.into())
}

/// Get the main worktree root by examining the git common directory.
///
/// The git common directory (`git rev-parse --git-common-dir`) points to:
/// - For the main worktree: `.git` (relative) or the absolute path to `.git`
/// - For linked worktrees: The path to the main repo's `.git` directory
///
/// The main worktree root is the parent of the git common directory.
fn get_main_worktree_root(path: &AbsoluteSystemPath) -> Result<AbsoluteSystemPathBuf, Error> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::git_error(format!(
            "git rev-parse --git-common-dir failed: {stderr}"
        )));
    }

    let git_common_dir = String::from_utf8(output.stdout)?.trim().to_string();

    // The git-common-dir output may be relative or absolute
    let git_common_path = if std::path::Path::new(&git_common_dir).is_absolute() {
        AbsoluteSystemPathBuf::try_from(git_common_dir.as_str())?
    } else {
        // Relative path - resolve it relative to the current directory
        // Use std::path to handle complex relative paths (e.g., "../../../.git")
        let resolved = path.as_std_path().join(&git_common_dir);
        AbsoluteSystemPathBuf::try_from(resolved.as_path())?.to_realpath()?
    };

    // The main worktree root is the parent of the .git directory
    // Handle both bare repos and regular repos
    git_common_path
        .parent()
        .map(|p| p.to_owned())
        .ok_or_else(|| Error::git_error("git common dir has no parent directory"))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

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
        assert!(!info.is_linked_worktree());
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
}
