use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

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
    from_commit: Option<&str>,
    to_commit: &str,
) -> Result<HashSet<String>, Error> {
    // Initialize repository at repo root
    let repo = Repository::open(&git_root)?;
    let git_root = AbsoluteSystemPathBuf::new(git_root)?;
    let turbo_root = AbsoluteSystemPathBuf::new(turbo_root)?;
    let anchored_turbo_root = git_root.anchor(&turbo_root)?;

    let mut files = HashSet::new();
    add_changed_files_from_to_commit_to_working_tree(
        &git_root,
        &repo,
        &anchored_turbo_root,
        &turbo_root,
        to_commit,
        &mut files,
    )?;
    add_changed_files_from_unstaged_changes(
        &git_root,
        &repo,
        &anchored_turbo_root,
        &turbo_root,
        &mut files,
    )?;
    if let Some(from_commit) = from_commit {
        add_changed_files_from_commits(
            &git_root,
            &repo,
            &anchored_turbo_root,
            &turbo_root,
            &mut files,
            from_commit,
            to_commit,
        )?;
    }

    Ok(files)
}

fn reanchor_path_from_git_root_to_turbo_root(
    git_root: &AbsoluteSystemPathBuf,
    turbo_root: &AbsoluteSystemPathBuf,
    path: &Path,
) -> Result<AnchoredSystemPathBuf, Error> {
    let anchored_to_git_root_file_path: AnchoredSystemPathBuf = path.try_into()?;
    let absolute_file_path = git_root.resolve(&anchored_to_git_root_file_path);
    let anchored_to_turbo_root_file_path = turbo_root.anchor(&absolute_file_path)?;
    Ok(anchored_to_turbo_root_file_path)
}

// Equivalent of `git diff --name-only <to_commit> -- <turbo_root>
fn add_changed_files_from_to_commit_to_working_tree(
    git_root: &AbsoluteSystemPathBuf,
    repo: &Repository,
    anchored_turbo_root: &AnchoredSystemPathBuf,
    turbo_root: &AbsoluteSystemPathBuf,
    to_commit: &str,
    files: &mut HashSet<String>,
) -> Result<(), Error> {
    let to_commit_ref = repo.revparse_single(to_commit)?;
    let to_commit = to_commit_ref.peel_to_commit()?;
    let to_tree = to_commit.tree()?;
    let mut options = DiffOptions::new();
    options.pathspec(anchored_turbo_root.to_str()?);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&to_tree), Some(&mut options))?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(file_path) = file.path() {
            let anchored_to_turbo_root_file_path =
                reanchor_path_from_git_root_to_turbo_root(git_root, turbo_root, file_path)?;
            files.insert(anchored_to_turbo_root_file_path.to_str()?.to_string());
        }
    }

    Ok(())
}

// Equivalent of `git ls-files --other --exclude-standard -- <turbo_root>`
fn add_changed_files_from_unstaged_changes(
    git_root: &AbsoluteSystemPathBuf,
    repo: &Repository,
    anchored_turbo_root: &AnchoredSystemPathBuf,
    turbo_root: &AbsoluteSystemPathBuf,
    files: &mut HashSet<String>,
) -> Result<(), Error> {
    let mut options = DiffOptions::new();
    options.include_untracked(true);
    options.recurse_untracked_dirs(true);

    options.pathspec(anchored_turbo_root.to_str()?);

    let diff = repo.diff_index_to_workdir(None, Some(&mut options))?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(file_path) = file.path() {
            let anchored_to_turbo_root_file_path =
                reanchor_path_from_git_root_to_turbo_root(git_root, turbo_root, file_path)?;
            files.insert(anchored_to_turbo_root_file_path.to_str()?.to_string());
        }
    }

    Ok(())
}

// Equivalent of `git diff --name-only <from_commit>...<to_commit> --
// <turbo_root>` NOTE: This is **not** the same as
// `git diff --name-only <from_commit>..<to_commit> -- <turbo_root>`
// (note the triple dots vs double dots)
// The triple dot version is a diff of the most recent common ancestor (merge
// base) of <from_commit> and <to_commit> to <to_commit>, whereas the double dot
// version is a diff of <from_commit> to <to_commit>.
fn add_changed_files_from_commits(
    git_root: &AbsoluteSystemPathBuf,
    repo: &Repository,
    anchored_turbo_root: &AnchoredSystemPathBuf,
    turbo_root: &AbsoluteSystemPathBuf,
    files: &mut HashSet<String>,
    from_commit: &str,
    to_commit: &str,
) -> Result<(), Error> {
    let from_commit_ref = repo.revparse_single(from_commit)?;
    let to_commit_ref = repo.revparse_single(to_commit)?;
    let mergebase_of_from_and_to = repo.merge_base(from_commit_ref.id(), to_commit_ref.id())?;

    let mergebase_commit = repo.find_commit(mergebase_of_from_and_to)?;
    let to_commit = to_commit_ref.peel_to_commit()?;
    let mergebase_tree = mergebase_commit.tree()?;
    let to_tree = to_commit.tree()?;

    let mut options = DiffOptions::new();
    options.pathspec(anchored_turbo_root.to_str()?);

    let diff = repo.diff_tree_to_tree(Some(&mergebase_tree), Some(&to_tree), Some(&mut options))?;
    diff.print(DiffFormat::NameOnly, |_, _, _| true)?;

    for delta in diff.deltas() {
        let file = delta.old_file();
        if let Some(file_path) = file.path() {
            let anchored_to_turbo_root_file_path =
                reanchor_path_from_git_root_to_turbo_root(git_root, turbo_root, file_path)?;
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
/// * `git_root`: The root of the repository
/// * `from_commit`: The commit hash to checkout
/// * `file_path`: The path to the file
///
/// returns: Result<String, Error>
pub fn previous_content(
    git_root: PathBuf,
    from_commit: &str,
    file_path: PathBuf,
) -> Result<Vec<u8>, Error> {
    let repo = Repository::open(&git_root)?;
    let git_root = AbsoluteSystemPathBuf::new(dunce::canonicalize(git_root)?)?;
    let file_path = AbsoluteSystemPathBuf::new(dunce::canonicalize(file_path)?)?;
    let from_commit_ref = repo.revparse_single(from_commit)?;
    let from_commit = from_commit_ref.peel_to_commit()?;
    let from_tree = from_commit.tree()?;

    let relative_path = git_root.anchor(&file_path)?;

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
        process::Command,
    };

    use git2::{Oid, Repository};
    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::previous_content;
    use crate::{git::reanchor_path_from_git_root_to_turbo_root, Error};

    fn setup_repository() -> Result<(TempDir, Repository), Error> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init(repo_root.path())?;
        let mut config = repo.config()?;
        config.set_str("user.name", "test")?;
        config.set_str("user.email", "test@example.com")?;

        Ok((repo_root, repo))
    }

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

    fn commit_delete(repo: &Repository, path: &Path, previous_commit: Oid) -> Result<Oid, Error> {
        let mut index = repo.index()?;
        index.remove_path(path)?;
        let tree_oid = index.write_tree()?;
        index.write()?;
        let tree = repo.find_tree(tree_oid)?;
        let previous_commit = repo.find_commit(previous_commit)?;

        Ok(repo.commit(
            Some("HEAD"),
            &repo.signature()?,
            &repo.signature()?,
            "Commit",
            &tree,
            std::slice::from_ref(&&previous_commit),
        )?)
    }

    fn add_files_from_stdout(
        files: &mut HashSet<String>,
        git_root: &AbsoluteSystemPathBuf,
        turbo_root: &AbsoluteSystemPathBuf,
        stdout: Vec<u8>,
    ) {
        let stdout = String::from_utf8(stdout).unwrap();
        for line in stdout.lines() {
            let path = Path::new(line);
            let anchored_to_turbo_root_file_path =
                reanchor_path_from_git_root_to_turbo_root(git_root, turbo_root, path).unwrap();
            files.insert(
                anchored_to_turbo_root_file_path
                    .to_str()
                    .unwrap()
                    .to_string(),
            );
        }
    }

    // An implementation that shells out to the git command like the old Go version
    fn expected_changed_files(
        git_root: &Path,
        turbo_root: &Path,
        from_commit: Option<&str>,
        to_commit: &str,
    ) -> Result<HashSet<String>, Error> {
        let git_root = AbsoluteSystemPathBuf::new(dunce::canonicalize(git_root)?)?;
        let turbo_root = AbsoluteSystemPathBuf::new(dunce::canonicalize(turbo_root)?)?;

        let mut files = HashSet::new();
        let output = Command::new("git")
            .arg("diff")
            .arg("--name-only")
            .arg(to_commit)
            .arg("--")
            .arg(turbo_root.to_str().unwrap())
            .current_dir(&git_root)
            .output()
            .expect("failed to execute process");

        add_files_from_stdout(&mut files, &git_root, &turbo_root, output.stdout);

        if let Some(from_commit) = from_commit {
            let output = Command::new("git")
                .arg("diff")
                .arg("--name-only")
                .arg(format!("{}...{}", from_commit, to_commit))
                .arg("--")
                .arg(turbo_root.to_str().unwrap())
                .current_dir(&git_root)
                .output()
                .expect("failed to execute process");

            add_files_from_stdout(&mut files, &git_root, &turbo_root, output.stdout);
        }

        let output = Command::new("git")
            .arg("ls-files")
            .arg("--other")
            .arg("--exclude-standard")
            .arg("--")
            .arg(turbo_root.to_str().unwrap())
            .current_dir(&git_root)
            .output()
            .expect("failed to execute process");

        add_files_from_stdout(&mut files, &git_root, &turbo_root, output.stdout);

        Ok(files)
    }

    // Calls git::changed_files then compares it against the expected version that
    // shells out to git
    fn changed_files(
        git_root: PathBuf,
        turbo_root: PathBuf,
        from_commit: Option<&str>,
        to_commit: &str,
    ) -> Result<HashSet<String>, Error> {
        let expected_files =
            expected_changed_files(&git_root, &turbo_root, from_commit, to_commit)?;
        let files =
            super::changed_files(git_root.clone(), turbo_root.clone(), from_commit, to_commit)?;

        assert_eq!(files, expected_files);

        Ok(files)
    }

    #[test]
    fn test_deleted_files() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository()?;

        let file = repo_root.path().join("foo.js");
        let file_path = Path::new("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_oid = commit_file(&repo, &file_path, None)?;

        fs::remove_file(&file)?;
        let _second_commit_oid = commit_delete(&repo, &file_path, first_commit_oid)?;

        let first_commit_sha = first_commit_oid.to_string();
        let git_root = repo_root.path().to_owned();
        let turborepo_root = repo_root.path().to_owned();
        let files = changed_files(git_root, turborepo_root, Some(&first_commit_sha), "HEAD")?;

        assert_eq!(files, HashSet::from(["foo.js".to_string()]));
        Ok(())
    }

    #[test]
    fn test_merge_base() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository()?;
        let first_file = repo_root.path().join("foo.js");
        fs::write(&first_file, "let z = 0;")?;
        // Create a base commit. This will *not* be the merge base
        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None)?;

        let second_file = repo_root.path().join("bar.js");
        fs::write(&second_file, "let y = 1;")?;
        // This commit will be the merge base
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid))?;

        let third_file = repo_root.path().join("baz.js");
        fs::write(&third_file, "let x = 2;")?;
        // Create a first commit off of merge base
        let third_commit_oid = commit_file(&repo, Path::new("baz.js"), Some(second_commit_oid))?;

        // Move head back to merge base
        repo.set_head_detached(second_commit_oid)?;
        let fourth_file = repo_root.path().join("qux.js");
        fs::write(&fourth_file, "let w = 3;")?;
        // Create a second commit off of merge base
        let fourth_commit_oid = commit_file(&repo, Path::new("qux.js"), Some(second_commit_oid))?;

        repo.set_head_detached(third_commit_oid)?;
        let merge_base = repo.merge_base(third_commit_oid, fourth_commit_oid)?;

        assert_eq!(merge_base, second_commit_oid);

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some(&third_commit_oid.to_string()),
            &fourth_commit_oid.to_string(),
        )?;

        assert_eq!(
            files,
            HashSet::from(["qux.js".to_string(), "baz.js".to_string()])
        );

        Ok(())
    }

    #[test]
    fn test_changed_files() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository()?;
        let mut index = repo.index()?;
        let turbo_root = repo_root.path();
        let file = repo_root.path().join("foo.js");
        fs::write(file, "let z = 0;")?;

        // First commit (we need a base commit to compare against)
        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None)?;

        // Now change another file
        let new_file = repo_root.path().join("bar.js");
        fs::write(new_file, "let y = 1;")?;

        // Test that uncommitted file is marked as changed
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            None,
            "HEAD",
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Add file to index
        index.add_path(Path::new("bar.js"))?;
        index.write()?;

        // Test that uncommitted file in index is still marked as changed
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            None,
            "HEAD",
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Now commit file
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid))?;

        // Test that only second file is marked as changed when we check commit range
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some(first_commit_oid.to_string().as_str()),
            second_commit_oid.to_string().as_str(),
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Create a file nested in subdir
        fs::create_dir_all(repo_root.path().join("subdir"))?;
        let new_file = repo_root.path().join("subdir").join("baz.js");
        fs::write(new_file, "let x = 2;")?;

        // Test that `turbo_root` filters out files not in the specified directory
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some(first_commit_oid.to_string().as_str()),
            second_commit_oid.to_string().as_str(),
        )?;
        assert_eq!(files, HashSet::from(["baz.js".to_string()]));

        Ok(())
    }

    #[test]
    fn test_changed_files_with_root_as_relative() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository()?;
        let file = repo_root.path().join("foo.js");
        fs::write(file, "let z = 0;")?;

        // First commit (we need a base commit to compare against)
        commit_file(&repo, Path::new("foo.js"), None)?;

        // Now change another file
        let new_file = repo_root.path().join("bar.js");
        fs::write(new_file, "let y = 1;")?;

        // Test that uncommitted file is marked as changed with the parameters that Go
        // will pass
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            None,
            "HEAD",
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        Ok(())
    }

    // Tests that we can use a subdir as the turbo_root path
    // (occurs when the monorepo is nested inside a subdirectory of git repository)
    #[test]
    fn test_changed_files_with_subdir_as_turbo_root() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository()?;

        fs::create_dir(repo_root.path().join("subdir"))?;
        // Create additional nested directory to test that we return a system path
        // and not a normalized unix path
        fs::create_dir(repo_root.path().join("subdir").join("src"))?;

        let file = repo_root.path().join("subdir").join("foo.js");
        fs::write(file, "let z = 0;")?;
        let first_commit = commit_file(&repo, Path::new("subdir/foo.js"), None)?;

        let new_file = repo_root.path().join("subdir").join("src").join("bar.js");
        fs::write(new_file, "let y = 1;")?;

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            None,
            "HEAD",
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

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some(first_commit.to_string().as_str()),
            repo.head()?.peel_to_commit()?.id().to_string().as_str(),
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
        let (repo_root, repo) = setup_repository()?;

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
        let (repo_root, repo) = setup_repository()?;

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
