use std::path::PathBuf;

use anyhow::Result;
use git2::{DiffFormat, DiffOptions, Oid};

pub fn changed_files(
    repo_root: PathBuf,
    from_commit: String,
    to_commit: String,
    include_untracked: bool,
    relative_to: Option<PathBuf>,
) -> Result<Vec<String>> {
    let repo = git2::Repository::open(&repo_root)?;
    let relative_to = relative_to.unwrap_or(repo_root);
    let from_commit_oid = Oid::from_str(&from_commit)?;
    let to_commit_oid = Oid::from_str(&to_commit)?;
    let from_commit = repo.find_commit(from_commit_oid)?;
    let to_commit = repo.find_commit(to_commit_oid)?;
    let from_tree = from_commit.tree()?;
    let to_tree = to_commit.tree()?;
    let diff = repo.diff_tree_to_tree(
        Some(&from_tree),
        Some(&to_tree),
        Some(DiffOptions::new().pathspec(relative_to)),
    )?;

    diff.print(DiffFormat::NameOnly, |_, _, _| true)?;

    let mut files = vec![];
    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(path) = file.path() {
            files.push(path.to_string_lossy().to_string());
        }
    }

    Ok(files)
}
