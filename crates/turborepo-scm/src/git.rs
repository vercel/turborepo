use std::{collections::HashSet, path::PathBuf};

use git2::{DiffFormat, DiffOptions, Repository};
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

use crate::Error;

/// Finds the changed files in a repository between index and working directory
/// (unstaged changes) and between two commits. Includes untracked files,
/// i.e. files not yet in git
///
/// # Arguments
///
/// * `repo_root`: The root of the repository. Guaranteed to be the root.
/// * `commit_range`: If Some, the range of commits that should be searched for
///   changes
/// * `monorepo_root`: The path to which the results should be relative. Must be
///   an absolute path
///
/// returns: Result<HashSet<String, RandomState>, Error>
pub fn changed_files(
    git_root: PathBuf,
    turbo_root: PathBuf,
    commit_range: Option<(&str, &str)>,
) -> Result<HashSet<String>, Error> {
    // Initialize repository at repo root
    let repo = Repository::open(&git_root)?;
    let git_root = AbsoluteSystemPathBuf::new(git_root)?;
    let turbo_root = AbsoluteSystemPathBuf::new(turbo_root)?;

    let mut files = HashSet::new();
    add_changed_files_from_unstaged_changes(&git_root, &repo, &turbo_root, &mut files)?;

    if let Some((from_commit, to_commit)) = commit_range {
        add_changed_files_from_commits(
            &git_root,
            &repo,
            &turbo_root,
            &mut files,
            from_commit,
            to_commit,
        )?;
    }

    Ok(files)
}

fn add_changed_files_from_unstaged_changes(
    repo_root: &AbsoluteSystemPathBuf,
    repo: &Repository,
    turbo_root: &AbsoluteSystemPathBuf,
    files: &mut HashSet<String>,
) -> Result<(), Error> {
    let mut options = DiffOptions::new();
    options.include_untracked(true);
    options.recurse_untracked_dirs(true);

    let anchored_turbo_root = repo_root.anchor(turbo_root)?;
    options.pathspec(anchored_turbo_root.to_str()?.to_string());

    let diff = repo.diff_index_to_workdir(None, Some(&mut options))?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(file_path) = file.path() {
            let anchored_to_repo_root_file_path: AnchoredSystemPathBuf = file_path.try_into()?;
            let absolute_file_path = repo_root.resolve(&anchored_to_repo_root_file_path);
            let anchored_to_turbo_root_file_path = turbo_root.anchor(&absolute_file_path)?;
            files.insert(anchored_to_turbo_root_file_path.to_str()?.to_string());
        }
    }

    Ok(())
}

fn add_changed_files_from_commits(
    repo_root: &AbsoluteSystemPathBuf,
    repo: &Repository,
    turbo_root: &AbsoluteSystemPathBuf,
    files: &mut HashSet<String>,
    from_commit: &str,
    to_commit: &str,
) -> Result<(), Error> {
    let from_commit_ref = repo.revparse_single(from_commit)?;
    let to_commit_ref = repo.revparse_single(to_commit)?;
    let from_commit = from_commit_ref.peel_to_commit()?;
    let to_commit = to_commit_ref.peel_to_commit()?;
    let from_tree = from_commit.tree()?;
    let to_tree = to_commit.tree()?;

    let mut options = DiffOptions::new();
    let anchored_turbo_root = repo_root.anchor(turbo_root)?;
    options.pathspec(anchored_turbo_root.to_str()?);

    let diff = repo.diff_tree_to_tree(Some(&from_tree), Some(&to_tree), Some(&mut options))?;
    diff.print(DiffFormat::NameOnly, |_, _, _| true)?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(file_path) = file.path() {
            let anchored_to_repo_root_file_path: AnchoredSystemPathBuf = file_path.try_into()?;
            let absolute_file_path = repo_root.resolve(&anchored_to_repo_root_file_path);
            let anchored_to_turbo_root_file_path = turbo_root.anchor(&absolute_file_path)?;
            files.insert(anchored_to_turbo_root_file_path.to_str()?.to_string());
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
    let repo = Repository::open(&repo_root)?;
    let repo_root = AbsoluteSystemPathBuf::new(dunce::canonicalize(repo_root)?)?;
    let file_path = AbsoluteSystemPathBuf::new(dunce::canonicalize(file_path)?)?;
    let from_commit_ref = repo.revparse_single(from_commit)?;
    let from_commit = from_commit_ref.peel_to_commit()?;
    let from_tree = from_commit.tree()?;

    let relative_path = repo_root.anchor(&file_path)?;

    let file = from_tree.get_path(relative_path.as_path())?;
    let blob = repo.find_blob(file.id())?;
    let content = blob.content();

    Ok(content.to_vec())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        env::set_current_dir,
        fs,
        path::{Path, PathBuf},
    };

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
        let turbo_root = repo_root.path();
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

        // Test that uncommitted file is marked as changed
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            None,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Now commit file
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid))?;

        // Test that only second file is marked as changed when we check commit range
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some((
                first_commit_oid.to_string().as_str(),
                second_commit_oid.to_string().as_str(),
            )),
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Create a file nested in subdir
        fs::create_dir_all(repo_root.path().join("subdir"))?;
        let new_file = repo_root.path().join("subdir").join("baz.js");
        fs::write(new_file, "let x = 2;")?;

        // Test that `turbo_root` filters out files not in the specified directory
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some((
                first_commit_oid.to_string().as_str(),
                second_commit_oid.to_string().as_str(),
            )),
        )?;
        assert_eq!(files, HashSet::from(["baz.js".to_string()]));

        Ok(())
    }

    #[test]
    fn test_changed_files_with_root_as_relative() -> Result<(), Error> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init(repo_root.path())?;
        let mut config = repo.config()?;
        config.set_str("user.name", "test")?;
        config.set_str("user.email", "test@example.com")?;
        let file = repo_root.path().join("foo.js");
        fs::write(file, "let z = 0;")?;

        // First commit (we need a base commit to compare against)
        commit_file(&repo, Path::new("foo.js"), None)?;

        // Now change another file
        let new_file = repo_root.path().join("bar.js");
        fs::write(new_file, "let y = 1;")?;

        // Test that uncommitted file is marked as changed with the parameters that Go
        // will pass
        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            None,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        Ok(())
    }

    // Tests that we can use a subdir as the turbo_root path
    // (occurs when the monorepo is nested inside a subdirectory of git repository)
    #[test]
    fn test_changed_files_with_subdir_as_turbo_root() -> Result<(), Error> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init(repo_root.path())?;
        let mut config = repo.config()?;
        config.set_str("user.name", "test")?;
        config.set_str("user.email", "test@example.com")?;

        fs::create_dir(repo_root.path().join("subdir"))?;
        // Create additional nested directory to test that we return a system path
        // and not a normalized unix path
        fs::create_dir(repo_root.path().join("subdir").join("src"))?;

        let file = repo_root.path().join("subdir").join("foo.js");
        fs::write(file, "let z = 0;")?;
        let first_commit = commit_file(&repo, Path::new("subdir/foo.js"), None)?;

        let new_file = repo_root.path().join("subdir").join("src").join("bar.js");
        fs::write(new_file, "let y = 1;")?;

        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            None,
        )?;

        #[cfg(unix)]
        {
            assert_eq!(files, HashSet::from(["src/bar.js".to_string()]));
        }

        #[cfg(windows)]
        {
            assert_eq!(files, HashSet::from(["src\\bar.js".to_string()]));
        }

        commit_file(&repo, Path::new("subdir/src/bar.js"), Some(first_commit))?;

        let files = super::changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some((
                first_commit.to_string().as_str(),
                repo.head()?.peel_to_commit()?.id().to_string().as_str(),
            )),
        )?;

        #[cfg(unix)]
        {
            assert_eq!(files, HashSet::from(["src/bar.js".to_string()]));
        }

        #[cfg(windows)]
        {
            assert_eq!(files, HashSet::from(["src\\bar.js".to_string()]));
        }

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

        set_current_dir(repo_root.path())?;

        // Check that relative paths work as well
        let content = previous_content(
            PathBuf::from("."),
            second_commit_oid.to_string().as_str(),
            PathBuf::from("./foo.js"),
        )?;
        assert_eq!(content, b"let z = 1;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            second_commit_oid.to_string().as_str(),
            PathBuf::from("./foo.js"),
        )?;
        assert_eq!(content, b"let z = 1;");

        Ok(())
    }

    #[test]
    fn test_revparse() -> Result<(), Error> {
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

        let revparsed_head = repo.revparse_single("HEAD")?;
        assert_eq!(revparsed_head.id(), second_commit_oid);
        let revparsed_head_minus_1 = repo.revparse_single("HEAD~1")?;
        assert_eq!(revparsed_head_minus_1.id(), first_commit_oid);

        Ok(())
    }
}
