use std::{collections::HashSet, path::PathBuf};

use dunce::canonicalize as fs_canonicalize;
use git2::{DiffFormat, DiffOptions, Oid, Repository};

use crate::Error;

/// Finds the changed files in a repository between index and working directory
/// (unstaged changes) and between two commits.
///
/// # Arguments
///
/// * `repo_root`: The root of the repository.
/// * `commit_range`: If Some, the range of commits that should be searched for
///   changes
/// * `include_untracked`: If true, untracked files will be included in the
///   result, i.e. files not yet in git.
/// * `relative_to`: If Some, only the results relative to this path will be
///   returned
///
/// returns: Result<HashSet<String, RandomState>, Error>
pub fn changed_files(
    repo_root: PathBuf,
    commit_range: Option<(&str, &str)>,
    include_untracked: bool,
    relative_to: Option<&str>,
) -> Result<HashSet<String>, Error> {
    let repo = Repository::open(repo_root)?;
    let mut files = HashSet::new();
    add_changed_files_from_unstaged_changes(&repo, &mut files, relative_to, include_untracked)?;

    if let Some((from_commit, to_commit)) = commit_range {
        add_changed_files_from_commits(&repo, &mut files, relative_to, from_commit, to_commit)?;
    }

    Ok(files)
}

fn add_changed_files_from_unstaged_changes(
    repo: &Repository,
    files: &mut HashSet<String>,
    relative_to: Option<&str>,
    include_untracked: bool,
) -> Result<(), Error> {
    let mut options = DiffOptions::new();
    options.include_untracked(include_untracked);
    options.recurse_untracked_dirs(include_untracked);
    relative_to.map(|relative_to| options.pathspec(relative_to));

    let diff = repo.diff_index_to_workdir(None, Some(&mut options))?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(path) = file.path() {
            // NOTE: In the original Go code, `Rel` works even if the base path is not a
            // prefix of the original path. In Rust, `strip_prefix` returns an
            // error if the base path is not a prefix of the original path.
            // However since we're passing a pathspec to `git2` we know that the
            // base path is a prefix of the original path.
            let path = relative_to.map_or(path, |relative_to| {
                path.strip_prefix(relative_to).unwrap_or(path)
            });
            files.insert(
                path.to_str()
                    .ok_or_else(|| Error::NonUtf8Path(path.to_path_buf()))?
                    .to_string(),
            );
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
) -> Result<(), Error> {
    let from_commit_oid = Oid::from_str(from_commit)?;
    let to_commit_oid = Oid::from_str(to_commit)?;
    let from_commit = repo.find_commit(from_commit_oid)?;
    let to_commit = repo.find_commit(to_commit_oid)?;
    let from_tree = from_commit.tree()?;
    let to_tree = to_commit.tree()?;
    let mut options = relative_to.map(|relative_to| {
        let mut options = DiffOptions::new();
        options.pathspec(relative_to);
        options
    });

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

/// Finds the content of a file at a previous commit. Assumes file is in a git
/// repository
///
/// # Arguments
///
/// * `repo_root`: The root of the repository
/// * `from_commit`: The commit hash to checkout
/// * `file_path`: The path to the file
///
/// returns: Result<String, Error>
pub fn previous_content(
    repo_root: PathBuf,
    from_commit: &str,
    file_path: PathBuf,
) -> Result<Vec<u8>, Error> {
    let repo = Repository::open(repo_root)?;
    let from_commit_oid = Oid::from_str(from_commit)?;
    let from_commit = repo.find_commit(from_commit_oid)?;
    let from_tree = from_commit.tree()?;

    // Canonicalize so strip_prefix works properly
    let file_path = fs_canonicalize(file_path)?;
    let repo_dir = fs_canonicalize(repo.workdir().ok_or(Error::RepositoryNotFound)?)?;

    let relative_path = file_path.strip_prefix(repo_dir).unwrap_or(&file_path);

    let file = from_tree.get_path(relative_path)?;
    let blob = repo.find_blob(file.id())?;
    let content = blob.content();

    Ok(content.to_vec())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs, path::Path};

    use git2::{Oid, Repository};

    use super::previous_content;
    use crate::Error;

    fn commit_file(
        repo: &Repository,
        path: &Path,
        previous_commit: Option<Oid>,
    ) -> Result<Oid, Error> {
        let mut index = repo.index()?;
        index.add_path(path)?;
        let tree_oid = index.write_tree()?;
        index.write()?;
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
    fn test_changed_files() -> Result<(), Error> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init(repo_root.path())?;
        let mut config = repo.config()?;
        config.set_str("user.name", "test")?;
        config.set_str("user.email", "test@example.com")?;
        let file = repo_root.path().join("foo.js");
        fs::write(file, "let z = 0;")?;

        // First commit (we need a base commit to compare against)
        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None)?;

        // Now change another file
        let new_file = repo_root.path().join("bar.js");
        fs::write(new_file, "let y = 1;")?;

        // Test that uncommitted file is marked as changed with `include_untracked`
        let files = super::changed_files(repo_root.path().to_path_buf(), None, true, None)?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Test that uncommitted file is *not* marked as changed without
        // `include_untracked`
        let files = super::changed_files(repo_root.path().to_path_buf(), None, false, None)?;
        assert_eq!(files, HashSet::from([]));

        // Now commit file
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid))?;

        // Test that only second file is marked as changed when we check commit range
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            Some((
                first_commit_oid.to_string().as_str(),
                second_commit_oid.to_string().as_str(),
            )),
            false,
            None,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Create a file nested in subdir
        fs::create_dir_all(repo_root.path().join("subdir"))?;
        let new_file = repo_root.path().join("subdir").join("baz.js");
        fs::write(new_file, "let x = 2;")?;

        // Test that `relative_to` filters out files not in the specified directory
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            Some((
                first_commit_oid.to_string().as_str(),
                second_commit_oid.to_string().as_str(),
            )),
            true,
            Some("subdir"),
        )?;
        assert_eq!(files, HashSet::from(["baz.js".to_string()]));

        Ok(())
    }

    #[test]
    fn test_previous_content() -> Result<(), Error> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init(repo_root.path())?;
        let mut config = repo.config()?;
        config.set_str("user.name", "test")?;
        config.set_str("user.email", "test@example.com")?;

        let file = repo_root.path().join("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None)?;
        fs::write(&file, "let z = 1;")?;
        let second_commit_oid = commit_file(&repo, Path::new("foo.js"), Some(first_commit_oid))?;

        let content = previous_content(
            repo_root.path().to_path_buf(),
            first_commit_oid.to_string().as_str(),
            file.clone(),
        )?;
        assert_eq!(content, b"let z = 0;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            second_commit_oid.to_string().as_str(),
            file,
        )?;
        assert_eq!(content, b"let z = 1;");

        Ok(())
    }
}
