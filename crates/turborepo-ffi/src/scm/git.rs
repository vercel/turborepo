use std::{collections::HashSet, path::PathBuf};

use anyhow::Result;
use git2::{DiffFormat, DiffOptions, Oid, Repository};

pub fn changed_files(
    repo_root: PathBuf,
    from_commit: Option<&str>,
    to_commit: &str,
    include_untracked: bool,
    relative_to: Option<&str>,
) -> Result<HashSet<String>> {
    let repo = Repository::open(&repo_root)?;
    let mut files = HashSet::new();
    add_changed_files_from_unstaged_changes(
        &repo,
        &mut files,
        relative_to.as_deref(),
        include_untracked,
    )?;
    if let Some(from_commit) = from_commit {
        add_changed_files_from_commits(
            &repo,
            &mut files,
            relative_to.as_deref(),
            from_commit,
            to_commit,
        )?;
    }

    Ok(files)
}

fn add_changed_files_from_unstaged_changes(
    repo: &Repository,
    files: &mut HashSet<String>,
    relative_to: Option<&str>,
    include_untracked: bool,
) -> Result<()> {
    let head = repo.head()?;
    let head_tree = head.peel_to_commit()?.tree()?;
    let mut options = DiffOptions::new();
    options.include_untracked(include_untracked);
    options.recurse_untracked_dirs(include_untracked);
    relative_to.map(|relative_to| options.pathspec(relative_to));

    let diff = repo.diff_tree_to_workdir_with_index(Some(&head_tree), Some(&mut options))?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(path) = file.path() {
            files.insert(path.to_string_lossy().to_string());
        }
    }

    Ok(())
}

fn add_changed_files_from_commits(
    repo: &Repository,
    files: &mut HashSet<String>,
    relative_to: Option<&str>,
    from_commit: &str,
    to_commit: &str,
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
            files.insert(path.to_string_lossy().to_string());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs, path::Path};

    use anyhow::Result;
    use git2::{Oid, Repository};

    fn commit_file(repo: &Repository, path: &Path, previous_commit: Option<Oid>) -> Result<Oid> {
        let mut index = repo.index()?;
        index.add_path(path)?;
        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;
        let previous_commit = previous_commit
            .map(|oid| repo.find_commit(oid))
            .transpose()?;

        Ok(repo.commit(
            Some("HEAD"),
            &repo.signature()?,
            &repo.signature()?,
            "Commit",
            &tree,
            previous_commit
                .as_ref()
                .as_ref()
                .map(std::slice::from_ref)
                .unwrap_or_default(),
        )?)
    }

    #[test]
    fn test_changed_files() -> Result<()> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init(repo_root.path())?;
        let file = repo_root.path().join("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None)?;
        // Test that committed file is marked as changed
        let files =
            super::changed_files(repo_root.path().to_path_buf(), None, "HEAD", false, None)?;
        assert_eq!(files, HashSet::from(["foo.js".to_string()]));

        // Now change another file
        let new_file = repo_root.path().join("bar.js");
        fs::write(&new_file, "let y = 1;")?;

        // Test that uncommitted file is marked as changed with `include_untracked`
        let files = super::changed_files(repo_root.path().to_path_buf(), None, "HEAD", true, None)?;
        assert_eq!(
            files,
            HashSet::from(["bar.js".to_string(), "foo.js".to_string()])
        );

        // Test that uncommitted file is *not* marked as changed without
        // `include_untracked`
        let files =
            super::changed_files(repo_root.path().to_path_buf(), None, "HEAD", false, None)?;
        assert_eq!(files, HashSet::from(["foo.js".to_string()]));

        // Now commit file
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid))?;

        // Test that uncommitted file in index is marked as changed
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            Some(first_commit_oid.to_string().as_str()),
            second_commit_oid.to_string().as_str(),
            false,
            None,
        )?;
        assert_eq!(
            files,
            HashSet::from(["bar.js".to_string(), "foo.js".to_string()])
        );

        // Create a file nested in subdir
        fs::create_dir_all(repo_root.path().join("subdir"))?;
        let new_file = repo_root.path().join("subdir").join("baz.js");
        fs::write(new_file, "let x = 2;")?;

        // Test that `relative_to` filters out files not in the specified directory
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            Some(first_commit_oid.to_string().as_str()),
            second_commit_oid.to_string().as_str(),
            true,
            Some("subdir"),
        )?;
        assert_eq!(files, HashSet::from(["subdir/baz.js".to_string()]));

        Ok(())
    }
}
