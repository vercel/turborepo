use std::{backtrace::Backtrace, collections::HashSet, path::PathBuf, process::Command};

use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPath,
};

use crate::{Error, Git, SCM};

impl SCM {
    pub fn changed_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_commit: Option<&str>,
        to_commit: &str,
    ) -> Result<HashSet<AnchoredSystemPathBuf>, Error> {
        match self {
            Self::Git(git) => git.changed_files(turbo_root, from_commit, to_commit),
            Self::Manual => Err(Error::GitRequired(turbo_root.to_owned())),
        }
    }

    pub fn previous_content(
        &self,
        from_commit: &str,
        file_path: &AbsoluteSystemPath,
    ) -> Result<Vec<u8>, Error> {
        match self {
            Self::Git(git) => git.previous_content(from_commit, file_path),
            Self::Manual => Err(Error::GitRequired(file_path.to_owned())),
        }
    }
}

/// Finds the changed files in a repository between index and working directory
/// (unstaged changes) and between two commits. Includes untracked files,
/// i.e. files not yet in git.
///
/// We shell out to git instead of using a git2 library because git2 doesn't
/// support shallow clones, and therefore errors on repositories that
/// are shallow cloned.
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
    let git_root = AbsoluteSystemPath::from_std_path(&git_root)?;
    let scm = SCM::new(git_root);

    let turbo_root = AbsoluteSystemPathBuf::try_from(turbo_root.as_path())?;
    let files = scm.changed_files(&turbo_root, from_commit, to_commit)?;
    Ok(files
        .into_iter()
        .map(|f| f.to_string())
        .collect::<HashSet<_>>())
}

impl Git {
    fn changed_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_commit: Option<&str>,
        to_commit: &str,
    ) -> Result<HashSet<AnchoredSystemPathBuf>, Error> {
        let turbo_root_relative_to_git_root = self.root.anchor(turbo_root)?;
        let pathspec = turbo_root_relative_to_git_root.as_str();

        let mut files = HashSet::new();

        let output = self.execute_git_command(&["diff", "--name-only", to_commit], pathspec)?;

        self.add_files_from_stdout(&mut files, turbo_root, output);

        if let Some(from_commit) = from_commit {
            let output = self.execute_git_command(
                &[
                    "diff",
                    "--name-only",
                    &format!("{}...{}", from_commit, to_commit),
                ],
                pathspec,
            )?;

            self.add_files_from_stdout(&mut files, turbo_root, output);
        }

        let output =
            self.execute_git_command(&["ls-files", "--others", "--exclude-standard"], pathspec)?;

        self.add_files_from_stdout(&mut files, turbo_root, output);

        Ok(files)
    }

    fn execute_git_command(&self, args: &[&str], pathspec: &str) -> Result<Vec<u8>, Error> {
        let mut command = Command::new(self.bin.as_std_path());
        command.args(args).current_dir(&self.root);

        if !pathspec.is_empty() {
            command.arg("--").arg(pathspec);
        }

        let output = command.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(Error::Git(stderr, Backtrace::capture()))
        } else {
            Ok(output.stdout)
        }
    }

    fn add_files_from_stdout(
        &self,
        files: &mut HashSet<AnchoredSystemPathBuf>,
        turbo_root: &AbsoluteSystemPath,
        stdout: Vec<u8>,
    ) {
        let stdout = String::from_utf8(stdout).unwrap();
        for line in stdout.lines() {
            let path = RelativeUnixPath::new(line).unwrap();
            let anchored_to_turbo_root_file_path = self
                .reanchor_path_from_git_root_to_turbo_root(turbo_root, path)
                .unwrap();
            files.insert(anchored_to_turbo_root_file_path);
        }
    }

    fn reanchor_path_from_git_root_to_turbo_root(
        &self,
        turbo_root: &AbsoluteSystemPath,
        path: &RelativeUnixPath,
    ) -> Result<AnchoredSystemPathBuf, Error> {
        let absolute_file_path = self.root.join_unix_path(path)?;
        let anchored_to_turbo_root_file_path = turbo_root.anchor(&absolute_file_path)?;
        Ok(anchored_to_turbo_root_file_path)
    }

    fn previous_content(
        &self,
        from_commit: &str,
        file_path: &AbsoluteSystemPath,
    ) -> Result<Vec<u8>, Error> {
        let anchored_file_path = self.root.anchor(file_path)?;
        let mut command = Command::new(self.bin.as_std_path());
        let command = command
            .arg("show")
            .arg(format!("{}:{}", from_commit, anchored_file_path.as_str()))
            .current_dir(&self.root);

        let output = command.output()?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(Error::Git(
                String::from_utf8_lossy(&output.stderr).to_string(),
                Backtrace::capture(),
            ))
        }
    }
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
    file_path: String,
) -> Result<Vec<u8>, Error> {
    // If git root is not absolute, we error.
    let git_root = AbsoluteSystemPathBuf::try_from(git_root)?;
    let scm = SCM::new(&git_root);

    // However for file path we handle both absolute and relative paths
    // Note that we assume any relative file path is relative to the git root
    // FIXME: this is probably wrong. We should know the path to the lockfile
    // exactly
    let absolute_file_path = AbsoluteSystemPathBuf::from_unknown(&git_root, file_path);

    scm.previous_content(from_commit, &absolute_file_path)
}

#[cfg(test)]
mod tests {
    use std::{
        assert_matches::assert_matches, collections::HashSet, fs, path::Path, process::Command,
    };

    use git2::{Oid, Repository};
    use tempfile::TempDir;
    use turbopath::{AbsoluteSystemPathBuf, PathError};
    use which::which;

    use super::previous_content;
    use crate::{git::changed_files, Error};

    fn setup_repository() -> Result<(TempDir, Repository), Error> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init(repo_root.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        Ok((repo_root, repo))
    }

    fn commit_file(repo: &Repository, path: &Path, previous_commit: Option<Oid>) -> Oid {
        let mut index = repo.index().unwrap();
        index.add_path(path).unwrap();
        let tree_oid = index.write_tree().unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let previous_commit = previous_commit
            .map(|oid| repo.find_commit(oid))
            .transpose()
            .unwrap();

        repo.commit(
            Some("HEAD"),
            &repo.signature().unwrap(),
            &repo.signature().unwrap(),
            "Commit",
            &tree,
            previous_commit
                .as_ref()
                .as_ref()
                .map(std::slice::from_ref)
                .unwrap_or_default(),
        )
        .unwrap()
    }

    fn commit_delete(repo: &Repository, path: &Path, previous_commit: Oid) -> Oid {
        let mut index = repo.index().unwrap();
        index.remove_path(path).unwrap();
        let tree_oid = index.write_tree().unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let previous_commit = repo.find_commit(previous_commit).unwrap();

        repo.commit(
            Some("HEAD"),
            &repo.signature().unwrap(),
            &repo.signature().unwrap(),
            "Commit",
            &tree,
            std::slice::from_ref(&&previous_commit),
        )
        .unwrap()
    }

    #[test]
    fn test_shallow_clone() -> Result<(), Error> {
        let tmp_dir = tempfile::tempdir()?;

        let git_binary = which("git")?;
        let output = Command::new(git_binary)
            .args([
                "clone",
                "--depth",
                "2",
                "https://github.com/vercel/app-playground.git",
                tmp_dir.path().to_str().unwrap(),
            ])
            .output()?;
        assert!(output.status.success());

        assert!(changed_files(
            tmp_dir.path().to_owned(),
            tmp_dir.path().to_owned(),
            Some("HEAD~1"),
            "HEAD",
        )
        .is_ok());

        assert!(changed_files(
            tmp_dir.path().to_owned(),
            tmp_dir.path().to_owned(),
            None,
            "HEAD",
        )
        .is_ok());

        Ok(())
    }

    #[test]
    fn test_deleted_files() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository()?;

        let file = repo_root.path().join("foo.js");
        let file_path = Path::new("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_oid = commit_file(&repo, file_path, None);

        fs::remove_file(&file)?;
        let _second_commit_oid = commit_delete(&repo, file_path, first_commit_oid);

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
        fs::write(first_file, "let z = 0;")?;
        // Create a base commit. This will *not* be the merge base
        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None);

        let second_file = repo_root.path().join("bar.js");
        fs::write(second_file, "let y = 1;")?;
        // This commit will be the merge base
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid));

        let third_file = repo_root.path().join("baz.js");
        fs::write(third_file, "let x = 2;")?;
        // Create a first commit off of merge base
        let third_commit_oid = commit_file(&repo, Path::new("baz.js"), Some(second_commit_oid));

        // Move head back to merge base
        repo.set_head_detached(second_commit_oid).unwrap();
        let fourth_file = repo_root.path().join("qux.js");
        fs::write(fourth_file, "let w = 3;")?;
        // Create a second commit off of merge base
        let fourth_commit_oid = commit_file(&repo, Path::new("qux.js"), Some(second_commit_oid));

        repo.set_head_detached(third_commit_oid).unwrap();
        let merge_base = repo
            .merge_base(third_commit_oid, fourth_commit_oid)
            .unwrap();

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
        let mut index = repo.index().unwrap();
        let turbo_root = repo_root.path();
        let file = repo_root.path().join("foo.js");
        fs::write(file, "let z = 0;")?;

        // First commit (we need a base commit to compare against)
        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None);

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
        index.add_path(Path::new("bar.js")).unwrap();
        index.write().unwrap();

        // Test that uncommitted file in index is still marked as changed
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            None,
            "HEAD",
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Now commit file
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid));

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
        commit_file(&repo, Path::new("foo.js"), None);

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
        let first_commit = commit_file(&repo, Path::new("subdir/foo.js"), None);

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

        commit_file(&repo, Path::new("subdir/src/bar.js"), Some(first_commit));

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some(first_commit.to_string().as_str()),
            repo.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .id()
                .to_string()
                .as_str(),
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

        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let file = root.join_component("foo.js");
        file.create_with_contents("let z = 0;")?;

        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None);
        fs::write(&file, "let z = 1;")?;
        let second_commit_oid = commit_file(&repo, Path::new("foo.js"), Some(first_commit_oid));

        let content = previous_content(
            repo_root.path().to_path_buf(),
            first_commit_oid.to_string().as_str(),
            file.to_string(),
        )?;

        assert_eq!(content, b"let z = 0;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            second_commit_oid.to_string().as_str(),
            file.to_string(),
        )?;
        assert_eq!(content, b"let z = 1;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            second_commit_oid.to_string().as_str(),
            "foo.js".to_string(),
        )?;
        assert_eq!(content, b"let z = 1;");

        Ok(())
    }

    #[test]
    fn test_revparse() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository()?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let file = root.join_component("foo.js");
        file.create_with_contents("let z = 0;")?;

        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None);
        fs::write(&file, "let z = 1;")?;
        let second_commit_oid = commit_file(&repo, Path::new("foo.js"), Some(first_commit_oid));

        let revparsed_head = repo.revparse_single("HEAD").unwrap();
        assert_eq!(revparsed_head.id(), second_commit_oid);
        let revparsed_head_minus_1 = repo.revparse_single("HEAD~1").unwrap();
        assert_eq!(revparsed_head_minus_1.id(), first_commit_oid);

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some("HEAD^"),
            "HEAD",
        )?;
        assert_eq!(files, HashSet::from(["foo.js".to_string()]));

        let content = previous_content(repo_root.path().to_path_buf(), "HEAD^", file.to_string())?;
        assert_eq!(content, b"let z = 0;");

        let new_file = repo_root.path().join("bar.js");
        fs::write(new_file, "let y = 0;")?;
        let third_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(second_commit_oid));
        let third_commit = repo.find_commit(third_commit_oid).unwrap();
        repo.branch("release-1", &third_commit, false).unwrap();

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some("HEAD~1"),
            "release-1",
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        Ok(())
    }

    #[test]
    fn test_error_cases() -> Result<(), Error> {
        let repo_dir = tempfile::tempdir()?;
        let repo_does_not_exist = changed_files(
            repo_dir.path().to_path_buf(),
            repo_dir.path().to_path_buf(),
            None,
            "HEAD",
        );

        assert_matches!(repo_does_not_exist, Err(Error::GitRequired(_)));

        let (repo_root, _repo) = setup_repository()?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let commit_does_not_exist = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            None,
            "does-not-exist",
        );

        assert_matches!(commit_does_not_exist, Err(Error::Git(_, _)));

        let file_does_not_exist = previous_content(
            repo_root.path().to_path_buf(),
            "HEAD",
            root.join_component("does-not-exist").to_string(),
        );
        assert_matches!(file_does_not_exist, Err(Error::Git(_, _)));

        let turbo_root = tempfile::tempdir()?;
        let turbo_root_is_not_subdir_of_git_root = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.path().to_path_buf(),
            None,
            "HEAD",
        );

        assert_matches!(
            turbo_root_is_not_subdir_of_git_root,
            Err(Error::Path(PathError::NotParent(_, _), _))
        );

        Ok(())
    }
}
