use std::{
    backtrace::Backtrace,
    collections::HashSet,
    env::{self, VarError},
    fs::{self},
    path::PathBuf,
    process::Command,
};

use serde::Deserialize;
use tracing::warn;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPath,
};
use turborepo_ci::Vendor;

use crate::{Error, Git, SCM};

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidRange {
    pub from_ref: Option<String>,
    pub to_ref: Option<String>,
}

impl SCM {
    pub fn get_current_branch(&self, path: &AbsoluteSystemPath) -> Result<String, Error> {
        match self {
            Self::Git(git) => git.get_current_branch(),
            Self::Manual => Err(Error::GitRequired(path.to_owned())),
        }
    }

    pub fn get_current_sha(&self, path: &AbsoluteSystemPath) -> Result<String, Error> {
        match self {
            Self::Git(git) => git.get_current_sha(),
            Self::Manual => Err(Error::GitRequired(path.to_owned())),
        }
    }

    pub fn changed_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_commit: Option<&str>,
        to_commit: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<Result<HashSet<AnchoredSystemPathBuf>, InvalidRange>, Error> {
        fn unable_to_detect_range(
            error: impl std::error::Error,
            from_ref: Option<String>,
            to_ref: Option<String>,
        ) -> Result<Result<HashSet<AnchoredSystemPathBuf>, InvalidRange>, Error> {
            warn!(
                "unable to detect git range, assuming all files have changed: {}",
                error
            );
            Ok(Err(InvalidRange { from_ref, to_ref }))
        }
        match self {
            Self::Git(git) => {
                match git.changed_files(
                    turbo_root,
                    from_commit,
                    to_commit,
                    include_uncommitted,
                    merge_base,
                ) {
                    Ok(files) => Ok(Ok(files)),
                    Err(ref error @ Error::Git(ref message, _))
                        if allow_unknown_objects
                            && (message.contains("no merge base")
                                || message.contains("bad object")) =>
                    {
                        unable_to_detect_range(
                            error,
                            from_commit.map(|c| c.to_string()),
                            to_commit.map(|c| c.to_string()),
                        )
                    }
                    Err(Error::UnableToResolveRef) => unable_to_detect_range(
                        Error::UnableToResolveRef,
                        from_commit.map(|c| c.to_string()),
                        to_commit.map(|c| c.to_string()),
                    ),
                    Err(e) => Err(e),
                }
            }
            Self::Manual => Err(Error::GitRequired(turbo_root.to_owned())),
        }
    }

    pub fn previous_content(
        &self,
        from_commit: Option<&str>,
        file_path: &AbsoluteSystemPath,
    ) -> Result<Vec<u8>, Error> {
        match self {
            Self::Git(git) => git.previous_content(from_commit, file_path),
            Self::Manual => Err(Error::GitRequired(file_path.to_owned())),
        }
    }
}

const UNKNOWN_SHA: &str = "0000000000000000000000000000000000000000";

#[derive(Debug, Deserialize, Clone)]
struct GitHubCommit {
    id: String,
}

#[derive(Debug, Deserialize, Default)]
struct GitHubEvent {
    #[serde(default)]
    before: String,

    #[serde(default)]
    commits: Vec<GitHubCommit>,

    #[serde(default)]
    forced: bool,
}

impl GitHubEvent {
    fn get_parent_ref_of_first_commit(&self) -> Option<String> {
        if self.commits.is_empty() {
            // commits can be empty when you push a branch with no commits
            return None;
        }

        if self.commits.len() >= 2048 {
            // GitHub API limit for number of commits shown in this field
            return None;
        }

        // Extract the base ref from the push event
        let first_commit = self.commits.first()?;
        let id = &first_commit.id;
        Some(format!("{id}^"))
    }
}

#[derive(Debug)]
pub struct CIEnv {
    is_github_actions: bool,
    github_base_ref: Result<String, VarError>,
    github_event_path: Result<String, VarError>,
}

impl Default for CIEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl CIEnv {
    pub fn new() -> Self {
        Self {
            is_github_actions: Vendor::is("GitHub Actions"),
            github_base_ref: env::var("GITHUB_BASE_REF"),
            github_event_path: env::var("GITHUB_EVENT_PATH"),
        }
    }
    pub fn none() -> Self {
        Self {
            is_github_actions: false,
            github_base_ref: Err(VarError::NotPresent),
            github_event_path: Err(VarError::NotPresent),
        }
    }
}

impl Git {
    fn get_current_branch(&self) -> Result<String, Error> {
        let output = self.execute_git_command(&["branch", "--show-current"], "")?;
        let output = String::from_utf8(output)?;
        Ok(output.trim().to_owned())
    }

    fn get_current_sha(&self) -> Result<String, Error> {
        let output = self.execute_git_command(&["rev-parse", "HEAD"], "")?;
        let output = String::from_utf8(output)?;
        Ok(output.trim().to_owned())
    }

    /// for GitHub Actions environment variables, see: https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/store-information-in-variables#default-environment-variables
    pub fn get_github_base_ref(base_ref_env: CIEnv) -> Option<String> {
        // make sure we're running in a CI environment
        if !base_ref_env.is_github_actions {
            return None;
        }

        /*
         * The name of the base ref or target branch of the pull request in a
         * workflow run.
         *
         * This variable only has a value when the event that triggers a workflow run
         * is either `pull_request` or `pull_request_target`.
         * For example, `main`
         *
         * So environment variable is empty in a regular commit
         */
        if let Ok(pr) = base_ref_env.github_base_ref {
            if !pr.is_empty() {
                return Some(pr);
            }
        }

        // we must be in a push event
        // try reading from the GITHUB_EVENT_PATH file
        if let Ok(event_path) = base_ref_env.github_event_path {
            // Try to open the event file and read the contents
            let data = fs::read_to_string(event_path).ok()?;

            // Parse the JSON data from the file
            let json: GitHubEvent = serde_json::from_str(&data).ok()?;

            // Extract the base ref from the pull request event if available
            let base_ref = &json.before;

            // the base_ref will be UNKNOWN_SHA on first push
            // we also use this behavior in force pushes
            if base_ref == UNKNOWN_SHA || json.forced {
                return json.get_parent_ref_of_first_commit();
            }

            if base_ref.is_empty() {
                return None;
            }

            return Some(base_ref.to_string());
        }
        None
    }

    fn resolve_base(&self, base_override: Option<&str>, env: CIEnv) -> Result<String, Error> {
        if let Some(valid_from) = base_override {
            return Ok(valid_from.to_string());
        }

        if let Some(github_base_ref) = Self::get_github_base_ref(env) {
            // we don't fall through to checking against main or master
            // because at this point we know we're in a GITHUB CI environment
            // and we should really know by now what the base ref is
            // so it's better to just error if something went wrong
            return match self.execute_git_command(&["rev-parse", &github_base_ref], "") {
                Ok(_) => {
                    eprintln!("Resolved base ref from GitHub Actions event: {github_base_ref}");
                    Ok(github_base_ref)
                }
                Err(e) => {
                    eprintln!(
                        "Failed to resolve base ref '{github_base_ref}' from GitHub Actions \
                         event: {e}"
                    );
                    Err(Error::UnableToResolveRef)
                }
            };
        }

        let main_result = self.execute_git_command(&["rev-parse", "main"], "");
        if main_result.is_ok() {
            return Ok("main".to_string());
        }

        let master_result = self.execute_git_command(&["rev-parse", "master"], "");
        if master_result.is_ok() {
            return Ok("master".to_string());
        }
        Err(Error::UnableToResolveRef)
    }

    fn changed_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_commit: Option<&str>,
        to_commit: Option<&str>,
        include_uncommitted: bool,
        merge_base: bool,
    ) -> Result<HashSet<AnchoredSystemPathBuf>, Error> {
        let turbo_root_relative_to_git_root = self.root.anchor(turbo_root)?;
        let pathspec = turbo_root_relative_to_git_root.as_str();

        let mut files = HashSet::new();

        let valid_from = self.resolve_base(from_commit, CIEnv::new())?;

        // If a to commit is not specified for `diff-tree` it will change the comparison
        // to be between the provided commit and it's parent
        let to_commit = to_commit.unwrap_or("HEAD");
        let mut args = vec![
            "diff-tree",
            "-r",
            "--name-only",
            "--no-commit-id",
            &valid_from,
            to_commit,
        ];

        if merge_base {
            args.push("--merge-base");
        }

        let output = self.execute_git_command(&args, pathspec)?;
        self.add_files_from_stdout(&mut files, turbo_root, output);

        // We only care about non-tracked files if we haven't specified both ends up the
        // comparison
        if include_uncommitted {
            // Add untracked files or unstaged changes, i.e. files that are not in git at
            // all
            let ls_files_output = self.execute_git_command(
                &["ls-files", "--others", "--modified", "--exclude-standard"],
                pathspec,
            )?;
            self.add_files_from_stdout(&mut files, turbo_root, ls_files_output);
            // Include any files that have been staged, but not committed
            let diff_output =
                self.execute_git_command(&["diff", "--name-only", "--cached"], pathspec)?;
            self.add_files_from_stdout(&mut files, turbo_root, diff_output);
        }

        Ok(files)
    }

    fn execute_git_command(&self, args: &[&str], pathspec: &str) -> Result<Vec<u8>, Error> {
        let mut command = Command::new(self.bin.as_std_path());
        command
            .args(args)
            .current_dir(&self.root)
            .env("GIT_OPTIONAL_LOCKS", "0");

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
        let absolute_file_path = self.root.join_unix_path(path);
        let anchored_to_turbo_root_file_path = turbo_root.anchor(&absolute_file_path)?;
        Ok(anchored_to_turbo_root_file_path)
    }

    fn previous_content(
        &self,
        from_commit: Option<&str>,
        file_path: &AbsoluteSystemPath,
    ) -> Result<Vec<u8>, Error> {
        let anchored_file_path = self.root.anchor(file_path)?;
        let valid_from = self.resolve_base(from_commit, CIEnv::new())?;
        let arg = format!("{}:{}", valid_from, anchored_file_path.as_str());

        self.execute_git_command(&["show", &arg], "")
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
    from_commit: Option<&str>,
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
        assert_matches::assert_matches,
        collections::HashSet,
        env::VarError,
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use git2::{Oid, Repository, RepositoryInitOptions};
    use tempfile::{NamedTempFile, TempDir};
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError};
    use which::which;

    use super::{previous_content, CIEnv, InvalidRange};
    use crate::{
        git::{GitHubCommit, GitHubEvent},
        Error, Git, SCM,
    };

    fn setup_repository(
        init_opts: Option<&RepositoryInitOptions>,
    ) -> Result<(TempDir, Repository), Error> {
        let repo_root = tempfile::tempdir()?;
        let repo = Repository::init_opts(
            repo_root.path(),
            init_opts.unwrap_or(&RepositoryInitOptions::new()),
        )
        .unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        Ok((repo_root, repo))
    }

    fn changed_files(
        git_root: PathBuf,
        turbo_root: PathBuf,
        from_commit: Option<&str>,
        to_commit: Option<&str>,
        include_uncommitted: bool,
    ) -> Result<HashSet<String>, Error> {
        let git_root = AbsoluteSystemPath::from_std_path(&git_root)?;
        let scm = SCM::new(git_root);

        let turbo_root = AbsoluteSystemPathBuf::try_from(turbo_root.as_path())?;
        // Replicating the `--filter` behavior where we only do a merge base
        // if both ends of the git range are specified.
        let merge_base = to_commit.is_some();
        let Ok(files) = scm.changed_files(
            &turbo_root,
            from_commit,
            to_commit,
            include_uncommitted,
            false,
            merge_base,
        )?
        else {
            unreachable!("changed_files should always return Some");
        };

        Ok(files
            .into_iter()
            .map(|f| f.to_string())
            .collect::<HashSet<_>>())
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

    fn commit_rename(repo: &Repository, source: &Path, dest: &Path, previous_commit: Oid) -> Oid {
        let mut index = repo.index().unwrap();
        index.remove_path(source).unwrap();
        index.add_path(dest).unwrap();
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
            Some("HEAD"),
            false,
        )
        .is_ok());

        assert!(changed_files(
            tmp_dir.path().to_owned(),
            tmp_dir.path().to_owned(),
            Some("HEAD"),
            None,
            true,
        )
        .is_ok());

        Ok(())
    }

    #[test]
    fn test_deleted_files() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository(None)?;

        let file = repo_root.path().join("foo.js");
        let file_path = Path::new("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_oid = commit_file(&repo, file_path, None);

        fs::remove_file(&file)?;
        let _second_commit_oid = commit_delete(&repo, file_path, first_commit_oid);

        let first_commit_sha = first_commit_oid.to_string();
        let git_root = repo_root.path().to_owned();
        let turborepo_root = repo_root.path().to_owned();
        let files = changed_files(
            git_root,
            turborepo_root,
            Some(&first_commit_sha),
            Some("HEAD"),
            false,
        )?;

        assert_eq!(files, HashSet::from(["foo.js".to_string()]));
        Ok(())
    }

    #[test]
    fn test_renamed_files() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository(None)?;

        let file = repo_root.path().join("foo.js");
        let file_path = Path::new("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_oid = commit_file(&repo, file_path, None);

        fs::rename(file, repo_root.path().join("bar.js")).unwrap();

        let new_file_path = Path::new("bar.js");
        let _second_commit_oid = commit_rename(&repo, file_path, new_file_path, first_commit_oid);

        let first_commit_sha = first_commit_oid.to_string();
        let git_root = repo_root.path().to_owned();
        let turborepo_root = repo_root.path().to_owned();
        let files = changed_files(
            git_root,
            turborepo_root,
            Some(&first_commit_sha),
            Some("HEAD"),
            false,
        )?;

        assert_eq!(
            files,
            HashSet::from(["foo.js".to_string(), "bar.js".to_string()])
        );
        Ok(())
    }
    #[test]
    fn test_merge_base() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository(None)?;
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
            Some(&fourth_commit_oid.to_string()),
            false,
        )?;

        assert_eq!(
            files,
            HashSet::from(["qux.js".to_string(), "baz.js".to_string()])
        );

        Ok(())
    }

    #[test]
    fn test_changed_files() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository(None)?;
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
            Some("HEAD"),
            None,
            true,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Test that uncommitted file in index is not marked as changed when not
        // checking uncommitted
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some("HEAD"),
            None,
            false,
        )?;
        assert_eq!(files, HashSet::new());

        // Test that uncommitted file in index is marked as changed when
        // checking uncommitted
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some("HEAD"),
            None,
            true,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Add file to index
        index.add_path(Path::new("bar.js")).unwrap();
        index.write().unwrap();

        // Test that uncommitted file in index is not marked as changed when not
        // checking uncommitted
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some("HEAD"),
            None,
            false,
        )?;
        assert_eq!(files, HashSet::new());

        // Test that uncommitted file in index is still marked as changed
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some("HEAD"),
            None,
            true,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Now commit file
        let second_commit_oid = commit_file(&repo, Path::new("bar.js"), Some(first_commit_oid));

        // Test that only second file is marked as changed when we check commit range
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some(first_commit_oid.to_string().as_str()),
            Some(second_commit_oid.to_string().as_str()),
            false,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Create a file nested in subdir
        fs::create_dir_all(repo_root.path().join("subdir"))?;
        let new_file = repo_root.path().join("subdir").join("baz.js");
        fs::write(new_file, "let x = 2;")?;

        // The new directory and files are not yet committed, they shouldn't show up.
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some(first_commit_oid.to_string().as_str()),
            Some(second_commit_oid.to_string().as_str()),
            false,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Since we are only specifying the first commit, the new file should show up
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some(second_commit_oid.to_string().as_str()),
            None,
            true,
        )?;
        assert_eq!(
            files,
            HashSet::from([format!("subdir{}baz.js", std::path::MAIN_SEPARATOR)])
        );

        // Commit the new file so it shows up in the changed files
        let third_commit_oid = commit_file(
            &repo,
            &Path::new("subdir").join("baz.js"),
            Some(second_commit_oid),
        );

        // Test that `turbo_root` filters out files not in the specified directory
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some(first_commit_oid.to_string().as_str()),
            Some(third_commit_oid.to_string().as_str()),
            false,
        )?;
        assert_eq!(files, HashSet::from(["baz.js".to_string()]));

        Ok(())
    }

    #[test]
    fn test_changed_files_with_root_as_relative() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository(None)?;
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
            Some("HEAD"),
            None,
            true,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        Ok(())
    }

    // Tests that we can use a subdir as the turbo_root path
    // (occurs when the monorepo is nested inside a subdirectory of git repository)
    #[test]
    fn test_changed_files_with_subdir_as_turbo_root() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository(None)?;

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
            Some("HEAD"),
            None,
            true,
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
            Some(
                repo.head()
                    .unwrap()
                    .peel_to_commit()
                    .unwrap()
                    .id()
                    .to_string()
                    .as_str(),
            ),
            false,
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
        let (repo_root, repo) = setup_repository(None)?;

        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let file = root.join_component("foo.js");
        file.create_with_contents("let z = 0;")?;

        let first_commit_oid = commit_file(&repo, Path::new("foo.js"), None);
        fs::write(&file, "let z = 1;")?;
        let second_commit_oid = commit_file(&repo, Path::new("foo.js"), Some(first_commit_oid));

        let content = previous_content(
            repo_root.path().to_path_buf(),
            Some(first_commit_oid.to_string().as_str()),
            file.to_string(),
        )?;

        assert_eq!(content, b"let z = 0;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            Some(second_commit_oid.to_string().as_str()),
            file.to_string(),
        )?;
        assert_eq!(content, b"let z = 1;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            Some(second_commit_oid.to_string().as_str()),
            "foo.js".to_string(),
        )?;
        assert_eq!(content, b"let z = 1;");

        Ok(())
    }

    #[test]
    fn test_revparse() -> Result<(), Error> {
        let (repo_root, repo) = setup_repository(None)?;
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
            Some("HEAD"),
            false,
        )?;
        assert_eq!(files, HashSet::from(["foo.js".to_string()]));

        let content = previous_content(
            repo_root.path().to_path_buf(),
            Some("HEAD^"),
            file.to_string(),
        )?;
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
            Some("release-1"),
            false,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        Ok(())
    }

    #[test_case(vec!["main"],                      None,            Some("main"))]
    #[test_case(vec!["master"],                    None,            Some("master"))]
    #[test_case(vec!["ziltoid"],                   None,            None)]
    #[test_case(vec!["ziltoid", "main"],           Some("ziltoid"), Some("ziltoid"))]
    #[test_case(vec!["ziltoid", "main"],           Some("main"),    Some("main"))]
    #[test_case(vec!["ziltoid", "main"],           None,            Some("main"))]
    #[test_case(vec!["ziltoid", "master"],         Some("ziltoid"), Some("ziltoid"))]
    #[test_case(vec!["ziltoid", "master"],         Some("master"),  Some("master"))]
    #[test_case(vec!["ziltoid", "master"],         None,            Some("master"))]
    #[test_case(vec!["ziltoid", "master", "main"], Some("ziltoid"), Some("ziltoid"))]
    #[test_case(vec!["ziltoid", "master", "main"], Some("master"),  Some("master"))]
    #[test_case(vec!["ziltoid", "master", "main"], Some("main"),    Some("main"))]
    #[test_case(vec!["ziltoid", "master", "main"], None,            Some("main"))]
    fn test_base_resolution(
        branches_to_create: Vec<&str>,
        target_branch: Option<&str>,
        expected: Option<&str>,
    ) -> Result<(), Error> {
        let mut repo_opts = RepositoryInitOptions::new();

        let (first_branch, remaining_branches) = branches_to_create.split_first().unwrap();

        let repo_init = repo_opts.initial_head(first_branch);
        let (repo_root, repo) = setup_repository(Some(repo_init))?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        // WARNING:
        // if you do not make a commit, git will show you that you have no branches.
        let file = root.join_component("todo.txt");
        file.create_with_contents("1. make async Rust good")?;
        let first_commit = commit_file(&repo, Path::new("todo.txt"), None);
        let commit = repo.find_commit(first_commit).unwrap();

        remaining_branches.iter().for_each(|branch| {
            repo.branch(branch, &commit, true).unwrap();
        });

        let thing = Git::find(&root).unwrap();
        let actual = thing.resolve_base(target_branch, CIEnv::none()).ok();

        assert_eq!(actual.as_deref(), expected);

        Ok(())
    }

    #[test]
    fn test_error_cases() -> Result<(), Error> {
        let repo_dir = tempfile::tempdir()?;
        let repo_does_not_exist = changed_files(
            repo_dir.path().to_path_buf(),
            repo_dir.path().to_path_buf(),
            Some("HEAD"),
            None,
            true,
        );

        assert_matches!(repo_does_not_exist, Err(Error::GitRequired(_)));

        let (repo_root, _repo) = setup_repository(None)?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let commit_does_not_exist = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some("does-not-exist"),
            None,
            true,
        );

        assert_matches!(commit_does_not_exist, Err(Error::Git(_, _)));

        let file_does_not_exist = previous_content(
            repo_root.path().to_path_buf(),
            Some("HEAD"),
            root.join_component("does-not-exist").to_string(),
        );
        assert_matches!(file_does_not_exist, Err(Error::Git(_, _)));

        let turbo_root = tempfile::tempdir()?;
        let turbo_root_is_not_subdir_of_git_root = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.path().to_path_buf(),
            Some("HEAD"),
            None,
            true,
        );

        assert_matches!(
            turbo_root_is_not_subdir_of_git_root,
            Err(Error::Path(PathError::NotParent(_, _), _))
        );

        Ok(())
    }

    #[test]
    fn test_changed_files_no_base() -> Result<(), Error> {
        let mut repo_opts = RepositoryInitOptions::new();

        let repo_init = repo_opts.initial_head("my-main");
        let (repo_root, repo) = setup_repository(Some(repo_init))?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        // WARNING:
        // if you do not make a commit, git will show you that you have no branches.
        let file = root.join_component("todo.txt");
        file.create_with_contents("1. explain why async Rust is good")?;
        let _first_commit = commit_file(&repo, Path::new("todo.txt"), None);

        let scm = SCM::new(&root);
        let actual = scm
            .changed_files(&root, None, Some("HEAD"), true, true, false)
            .unwrap();

        assert_eq!(
            actual,
            Err(InvalidRange {
                from_ref: None,
                to_ref: Some("HEAD".to_string()),
            })
        );

        Ok(())
    }

    struct TestCase {
        env: CIEnv,
        event_json: &'static str,
    }

    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Err(VarError::NotPresent),
            },
            event_json: r#""#,
        },
        None
        ; "GITHUB_BASE_REF and GITHUB_EVENT_PATH are not set"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Ok("".to_string()),
                github_event_path: Err(VarError::NotPresent),
            },
            event_json: r#""#,
        },
        None
        ; "GITHUB_BASE_REF is set to an empty string"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Ok("The choice is yours, and yours alone".to_string()),
                github_event_path: Err(VarError::NotPresent),
            },
            event_json: r#""#,
        },
        Some("The choice is yours, and yours alone")
        ; "GITHUB_BASE_REF is set to a non-empty string"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("Olmec refused to give up the location of the Shrine of the Silver Monkey".to_string()),
            },
            event_json: r#""#,
        },
        None
        ; "GITHUB_EVENT_PATH is set, but the file fails to open"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("the_room_of_the_three_gargoyles.json".to_string()),
            },
            event_json: r#"first you must pass the temple guards!"#,
        },
        None
        ; "GITHUB_EVENT_PATH is set, is not valid JSON"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("olmecs_temple.json".to_string()),
            },
            event_json: r#"{}"#,
        },
        None
        ; "no 'before' key in the JSON"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("olmecs_temple.json".to_string()),
            },
            event_json: r#"{"forced":true}"#,
        },
        None
        ; "force push"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("shrine_of_the_silver_monkey.json".to_string()),
            },
            event_json: r#"{"before":"e83c5163316f89bfbde7d9ab23ca2e25604af290"}"#,
        },
        Some("e83c5163316f89bfbde7d9ab23ca2e25604af290")
        ; "found a valid 'before' key in the JSON"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("shrine_of_the_silver_monkey.json".to_string()),
            },
            event_json: r#"{"before":"0000000000000000000000000000000000000000"}"#,
        },
        None
        ; "UNKNOWN_SHA but no commits found"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("shrine_of_the_silver_monkey.json".to_string()),
            },
            event_json: r#"{"before":"0000000000000000000000000000000000000000","commits":[]}"#,
        },
        None
        ; "empty commits"
    )]
    #[test_case(
        TestCase {
            env: CIEnv {
                is_github_actions: true,
                github_base_ref: Err(VarError::NotPresent),
                github_event_path: Ok("shrine_of_the_silver_monkey.json".to_string()),
            },
            event_json: r#"{"before":"0000000000000000000000000000000000000000","commits":[{"id":"yep"}]}"#,
        },
        Some("yep^")
        ; "first commit has a parent"
    )]
    fn test_get_github_base_ref(test_case: TestCase, expected: Option<&str>) -> Result<(), Error> {
        // note: we must bind here because otherwise the temporary file will be dropped
        let temp_file = if test_case.env.github_event_path.is_ok() {
            let temp_file = NamedTempFile::new().expect("Failed to create temporary file");
            fs::write(temp_file.path(), test_case.event_json)
                .expect("Failed to write to temporary file");
            Ok(temp_file)
        } else {
            Err(VarError::NotPresent)
        };

        let actual = Git::get_github_base_ref(CIEnv {
            is_github_actions: test_case.env.is_github_actions,
            github_base_ref: test_case.env.github_base_ref,
            github_event_path: temp_file
                .as_ref()
                .map(|p| p.path().to_str().unwrap().to_string())
                .map_err(|e| e.clone()),
        });
        assert_eq!(actual, expected.map(|s| s.to_string()));

        Ok(())
    }

    #[test]
    fn test_thousands_of_commits() {
        let commits = vec![
            GitHubCommit {
                id: "insert-famous-sha-here".to_string(),
            };
            2049 // 2049 is one over the limit
        ];

        let github_event = GitHubEvent {
            before: "".to_string(),
            commits,
            forced: false,
        };
        let actual = github_event.get_parent_ref_of_first_commit();

        assert_eq!(None, actual);
    }
}
