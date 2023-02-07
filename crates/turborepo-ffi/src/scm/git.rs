use std::path::PathBuf;

use anyhow::Result;
use git2::{DiffFormat, DiffOptions, Oid, Repository};

pub fn changed_files(
    repo_root: PathBuf,
    from_commit: Option<String>,
    to_commit: String,
    include_untracked: bool,
    relative_to: Option<String>,
) -> Result<Vec<String>> {
    let repo = git2::Repository::open(&repo_root)?;
    let mut files = Vec::new();
    add_changed_files_from_unstaged_changes(&repo, &mut files)?;
    if let Some(from_commit) = from_commit {
        add_changed_files_from_commits(&repo, &mut files, relative_to, from_commit, to_commit)?;
    }

    if include_untracked {
        todo!()
    }

    Ok(files)
}

fn add_changed_files_from_unstaged_changes(
    repo: &Repository,
    files: &mut Vec<String>,
) -> Result<()> {
    let head = repo.head()?;
    let head_tree = head.peel_to_commit()?.tree()?;
    let mut options = DiffOptions::new();
    options.include_untracked(true);
    let diff = repo.diff_tree_to_workdir_with_index(Some(&head_tree), Some(&mut options))?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(path) = file.path() {
            files.push(path.to_string_lossy().to_string());
        }
    }

    Ok(())
}

fn add_changed_files_from_commits(
    repo: &Repository,
    files: &mut Vec<String>,
    relative_to: Option<String>,
    from_commit: String,
    to_commit: String,
) -> Result<()> {
    let from_commit_oid = Oid::from_str(&from_commit)?;
    let to_commit_oid = Oid::from_str(&to_commit)?;
    let from_commit = repo.find_commit(from_commit_oid)?;
    let to_commit = repo.find_commit(to_commit_oid)?;
    let from_tree = from_commit.tree()?;
    let to_tree = to_commit.tree()?;
    let mut options = if let Some(relative_to) = relative_to {
        let mut options = DiffOptions::new();
        options.pathspec(relative_to);
        Some(options)
    } else {
        None
    };
    let diff = repo.diff_tree_to_tree(Some(&from_tree), Some(&to_tree), options.as_mut())?;
    diff.print(DiffFormat::NameOnly, |_, _, _| true)?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(path) = file.path() {
            files.push(path.to_string_lossy().to_string());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::scm::git::changed_files;

    #[test]
    fn test_changed_files() {
        let result = changed_files(
            "/Users/nicholas/repos/turbo".into(),
            "bf0980ed1d816568e8258c206ddf45d9a7e93c4f".into(),
            "c0d4854060b41269d8f3e9383a5af0539849c72f".into(),
            false,
            Some("crates/turborepo-ffi".into()),
        )
        .unwrap();
        println!("{:?}", result);
    }
}
