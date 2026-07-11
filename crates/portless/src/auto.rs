//! Automatic hostname, project, and Git worktree inference.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_DNS_LABEL_LENGTH: usize = 63;

/// A project name together with the inference source used to obtain it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferredName {
    pub name: String,
    pub source: &'static str,
}

/// A hostname prefix inferred from a linked Git worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreePrefix {
    pub prefix: String,
    pub source: &'static str,
}

/// Returned when no usable project name can be inferred.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectNameError;

impl fmt::Display for ProjectNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "Could not infer a project name from package.json, git root, or directory name",
        )
    }
}

impl std::error::Error for ProjectNameError {}

/// Truncate a DNS label to 63 characters, adding a deterministic SHA-256
/// suffix.
#[must_use]
pub fn truncate_label(label: &str) -> String {
    if label.chars().count() <= MAX_DNS_LABEL_LENGTH {
        return label.to_owned();
    }

    let hash = Sha256::digest(label.as_bytes());
    let suffix = hex_prefix(&hash, 6);
    let prefix: String = label
        .chars()
        .take(MAX_DNS_LABEL_LENGTH - 7)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned();
    format!("{prefix}-{suffix}")
}

/// Convert a string to a lower-case, single DNS label suitable for
/// `.localhost`.
#[must_use]
pub fn sanitize_for_hostname(name: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_was_hyphen = false;

    for character in name.chars().flat_map(char::to_lowercase) {
        let valid = character.is_ascii_lowercase() || character.is_ascii_digit();
        if valid {
            sanitized.push(character);
            previous_was_hyphen = false;
        } else if !previous_was_hyphen {
            sanitized.push('-');
            previous_was_hyphen = true;
        }
    }

    truncate_label(sanitized.trim_matches('-'))
}

/// Infer a project name from package metadata, the Git root, or the directory
/// name.
pub fn infer_project_name(cwd: impl AsRef<Path>) -> Result<InferredName, ProjectNameError> {
    let cwd = cwd.as_ref();
    if let Some(package_name) = find_package_json_name(cwd) {
        let name = sanitize_for_hostname(&package_name);
        if !name.is_empty() {
            return Ok(InferredName {
                name,
                source: "package.json",
            });
        }
    }

    if let Some(root) = find_git_root(cwd) {
        if let Some(file_name) = root.file_name().and_then(|name| name.to_str()) {
            let name = sanitize_for_hostname(file_name);
            if !name.is_empty() {
                return Ok(InferredName {
                    name,
                    source: "git root",
                });
            }
        }
    }

    if let Some(file_name) = cwd.file_name().and_then(|name| name.to_str()) {
        let name = sanitize_for_hostname(file_name);
        if !name.is_empty() {
            return Ok(InferredName {
                name,
                source: "directory name",
            });
        }
    }

    Err(ProjectNameError)
}

/// Infer a project name from the process's current directory.
pub fn infer_project_name_from_current_dir() -> Result<InferredName, ProjectNameError> {
    std::env::current_dir()
        .map_err(|_| ProjectNameError)
        .and_then(infer_project_name)
}

/// Detect a prefix for a linked worktree on a non-default branch.
#[must_use]
pub fn detect_worktree_prefix(cwd: impl AsRef<Path>) -> Option<WorktreePrefix> {
    let cwd = cwd.as_ref();
    match detect_worktree_via_cli(cwd) {
        CliDetection::Detected(prefix) => prefix,
        CliDetection::Unavailable => detect_worktree_via_filesystem(cwd),
    }
}

/// Detect a worktree prefix from the process's current directory.
#[must_use]
pub fn detect_worktree_prefix_from_current_dir() -> Option<WorktreePrefix> {
    std::env::current_dir()
        .ok()
        .and_then(detect_worktree_prefix)
}

fn hex_prefix(bytes: &[u8], digits: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 0xf) as usize] as char,
            ]
        })
        .take(digits)
        .collect()
}

fn find_package_json_name(start_dir: &Path) -> Option<String> {
    for dir in start_dir.ancestors() {
        let Ok(raw) = fs::read_to_string(dir.join("package.json")) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if let Some(name) = value
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            return Some(strip_scope(name).to_owned());
        }
    }
    None
}

fn strip_scope(name: &str) -> &str {
    if name.starts_with('@') {
        name.split_once('/').map_or(name, |(_, package)| package)
    } else {
        name
    }
}

fn find_git_root(start_dir: &Path) -> Option<PathBuf> {
    let output = git_output(start_dir, &["rev-parse", "--show-toplevel"]);
    if let Some(root) = output.filter(|root| !root.is_empty()) {
        return Some(PathBuf::from(root));
    }

    start_dir
        .ancestors()
        .find(|dir| fs::metadata(dir.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

fn branch_to_prefix(branch: &str) -> Option<String> {
    if branch.is_empty() || branch == "HEAD" || branch == "main" || branch == "master" {
        return None;
    }
    let last_segment = branch.rsplit('/').next().unwrap_or_default();
    let prefix = sanitize_for_hostname(last_segment);
    (!prefix.is_empty()).then_some(prefix)
}

enum CliDetection {
    Detected(Option<WorktreePrefix>),
    Unavailable,
}

fn detect_worktree_via_cli(cwd: &Path) -> CliDetection {
    let Some(worktrees) = git_output(cwd, &["worktree", "list", "--porcelain"]) else {
        return CliDetection::Unavailable;
    };
    if worktrees
        .lines()
        .filter(|line| line.starts_with("worktree "))
        .count()
        <= 1
    {
        return CliDetection::Detected(None);
    }

    let Some(git_dir) = git_output(cwd, &["rev-parse", "--git-dir"]) else {
        return CliDetection::Unavailable;
    };
    let Some(common_dir) = git_output(cwd, &["rev-parse", "--git-common-dir"]) else {
        return CliDetection::Unavailable;
    };
    if resolve_from(cwd, &git_dir) == resolve_from(cwd, &common_dir) {
        return CliDetection::Detected(None);
    }

    let Some(branch) = git_output(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return CliDetection::Unavailable;
    };
    CliDetection::Detected(branch_to_prefix(&branch).map(|prefix| WorktreePrefix {
        prefix,
        source: "git branch",
    }))
}

fn git_output(cwd: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(cwd)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn resolve_from(cwd: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn detect_worktree_via_filesystem(start_dir: &Path) -> Option<WorktreePrefix> {
    for dir in start_dir.ancestors() {
        let git_path = dir.join(".git");
        let Ok(metadata) = fs::metadata(&git_path) else {
            continue;
        };
        if metadata.is_dir() {
            return None;
        }
        if !metadata.is_file() {
            continue;
        }

        let content = fs::read_to_string(git_path).ok()?;
        let git_dir_value = content.trim().strip_prefix("gitdir:")?.trim();
        let git_dir = resolve_from(dir, git_dir_value);
        if git_dir.parent()?.file_name()?.to_str()? != "worktrees" {
            return None;
        }
        let branch = read_branch_from_head(&git_dir)?;
        return branch_to_prefix(&branch).map(|prefix| WorktreePrefix {
            prefix,
            source: "git branch",
        });
    }
    None
}

fn read_branch_from_head(git_dir: &Path) -> Option<String> {
    fs::read_to_string(git_dir.join("HEAD"))
        .ok()?
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn sanitizes_and_truncates_hostnames() {
        assert_eq!(sanitize_for_hostname("My___App!"), "my-app");
        assert_eq!(sanitize_for_hostname("@@@"), "");
        assert_eq!(sanitize_for_hostname(&"a".repeat(63)), "a".repeat(63));
        assert_eq!(
            sanitize_for_hostname(&"a".repeat(80)),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-0f45e8"
        );
    }

    #[test]
    fn infers_scoped_package_name_while_walking_up() {
        let temp = TempDir::new().expect("temp dir");
        let child = temp.path().join("src/components");
        fs::create_dir_all(&child).expect("child");
        fs::write(
            temp.path().join("package.json"),
            r#"{"name":"@scope/My_App"}"#,
        )
        .expect("package");

        assert_eq!(
            infer_project_name(child).expect("inferred"),
            InferredName {
                name: "my-app".to_owned(),
                source: "package.json"
            }
        );
    }

    #[test]
    fn filesystem_worktree_uses_last_branch_segment() {
        let temp = TempDir::new().expect("temp dir");
        let git_dir = temp.path().join("repo.git/worktrees/wt");
        fs::create_dir_all(&git_dir).expect("git dir");
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/feature/My_Branch\n").expect("head");
        fs::write(
            temp.path().join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .expect("git file");

        assert_eq!(
            detect_worktree_prefix(temp.path()),
            Some(WorktreePrefix {
                prefix: "my-branch".to_owned(),
                source: "git branch"
            })
        );
    }
}
