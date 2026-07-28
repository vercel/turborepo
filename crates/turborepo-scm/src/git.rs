use std::{
    backtrace::Backtrace,
    collections::HashSet,
    env::{self, VarError},
    fs,
    path::PathBuf,
    process::Command,
};

use serde::Deserialize;
use tracing::warn;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPath,
};
use turborepo_ci::Vendor;

use crate::{Error, GitRepo, RepoGitIndex, SCM};

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidRange {
    pub from_ref: Option<String>,
    pub to_ref: Option<String>,
}

impl SCM {
    pub fn get_current_sha(&self, path: &AbsoluteSystemPath) -> Result<String, Error> {
        match self {
            Self::Git(git) => git.get_current_sha(),
            Self::Manual => Err(Error::GitRequired(path.to_owned())),
        }
    }

    pub fn get_current_branch_and_sha(
        &self,
        _path: &AbsoluteSystemPath,
    ) -> (Option<String>, Option<String>) {
        match self {
            Self::Git(git) => (git.get_current_branch().ok(), git.get_current_sha().ok()),
            Self::Manual => (None, None),
        }
    }

    /// Compute a hash that summarizes all uncommitted changes in the working
    /// tree: staged changes, unstaged changes, and untracked files.
    /// Returns `None` for manual SCM mode, when the working tree is clean,
    /// or when a git command fails (errors are logged as warnings).
    pub fn get_dirty_hash(&self) -> Option<String> {
        match self {
            Self::Git(git) => git.get_dirty_hash(),
            Self::Manual => None,
        }
    }

    /// Compute a dirty hash from an already-built repo index instead of
    /// spawning `git status`. This preserves the same tracked-content diff
    /// input as [`Self::get_dirty_hash`], while reusing the untracked-file
    /// state Turbo already collected for file hashing.
    pub fn get_dirty_hash_from_repo_index(&self, repo_index: &RepoGitIndex) -> Option<String> {
        match self {
            Self::Git(git) => git.get_dirty_hash_from_repo_index(repo_index),
            Self::Manual => None,
        }
    }

    /// get the actual changed files between two git refs
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

impl GitRepo {
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

    fn validate_git_ref(git_ref: &str) -> Result<(), Error> {
        if git_ref.starts_with('-') {
            return Err(Error::InvalidGitRef(git_ref.to_string()));
        }

        Ok(())
    }

    /// Compute a hash summarizing all uncommitted state in the working tree.
    /// Uses `git status --porcelain -z` (which files are dirty/untracked) and
    /// `git diff HEAD` (the actual content changes for tracked files) as inputs
    /// to a SHA-256 hash. Returns `None` if the working tree is clean or if
    /// git commands fail (with a warning logged).
    ///
    /// The diff output is streamed through the hasher to avoid buffering
    /// arbitrarily large diffs into memory. `--no-ext-diff` and `--no-binary`
    /// ensure deterministic, bounded output regardless of user git config.
    ///
    /// Note: content of untracked files (not yet `git add`ed) is not included
    /// in the diff — only their filenames from `git status` contribute.
    fn get_dirty_hash(&self) -> Option<String> {
        use sha2::{Digest, Sha256};

        let status_output = match self.execute_git_command(&["status", "--porcelain", "-z"], "") {
            Ok(output) => output,
            Err(e) => {
                turborepo_log::warn(
                    turborepo_log::Source::turbo(turborepo_log::Subsystem::Scm),
                    format!("failed to get git status for dirty hash: {e}"),
                )
                .emit();
                return None;
            }
        };

        if status_output.is_empty() {
            return None;
        }

        let mut hasher = Sha256::new();
        hasher.update(&status_output);
        self.finish_dirty_hash(hasher, true)
    }

    fn get_dirty_hash_from_repo_index(&self, repo_index: &RepoGitIndex) -> Option<String> {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        let has_status = repo_index.append_dirty_status_to_hasher(&mut hasher);

        // If the repo index proved the working tree clean of unstaged
        // changes, and the index's cache-tree equals HEAD's tree (no staged
        // changes), then `git diff HEAD` is guaranteed to produce no output
        // — skip spawning it. On stat-stale indexes (snapshot-restored
        // workspaces) that subprocess re-hashes every tracked file with
        // sha1dc and costs seconds of serial wall time.
        let info_attributes_exists = self
            .root
            .join_components(&[".git", "info", "attributes"])
            .exists();
        if let Some((root, sensitivity)) =
            repo_index.tracked_diff_clean_root(info_attributes_exists)
        {
            let eol_ok = match sensitivity {
                crate::repo_index::EolSensitivity::ConfigIndependent => true,
                crate::repo_index::EolSensitivity::RequiresInertEolConversion => {
                    self.eol_conversion_inert()
                }
            };
            if eol_ok
                && self
                    .head_tree_oid()
                    .is_some_and(|head_tree| head_tree == root)
            {
                // The diff would have contributed nothing to the hasher, so
                // the resulting hash is identical to the diff-running path.
                if !has_status {
                    return None;
                }
                return Some(hex::encode(hasher.finalize()));
            }
        }

        self.finish_dirty_hash(hasher, has_status)
    }

    /// Returns true when git provably performs no eol conversion at checkin:
    /// `core.autocrlf` is unset or false, no `core.attributesFile` is
    /// configured, and no default global/system attribute files exist.
    /// Conservative on any failure.
    fn eol_conversion_inert(&self) -> bool {
        let Ok(output) = self.execute_git_command(&["config", "-z", "--list"], "") else {
            return false;
        };
        let mut autocrlf_ok = true;
        for kv in output.split(|b| *b == 0) {
            // `-z` output: `key\nvalue` per NUL-terminated record.
            let mut parts = kv.splitn(2, |b| *b == b'\n');
            let key = parts.next().unwrap_or_default();
            let value = parts.next().unwrap_or_default();
            match key {
                b"core.autocrlf" => {
                    // Last definition wins, mirroring git.
                    autocrlf_ok = value.eq_ignore_ascii_case(b"false");
                }
                b"core.attributesfile" => return false,
                _ => {}
            }
        }
        if !autocrlf_ok {
            return false;
        }

        // Default global attributes locations git consults when
        // core.attributesFile is unset.
        let xdg_attrs = std::env::var_os("XDG_CONFIG_HOME")
            .map(|base| {
                std::path::PathBuf::from(base)
                    .join("git")
                    .join("attributes")
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".config/git/attributes"))
            });
        if xdg_attrs.is_some_and(|p| p.exists()) {
            return false;
        }
        // System-wide attributes. The exact path depends on git's compiled
        // prefix; /etc/gitattributes is the common location.
        !std::path::Path::new("/etc/gitattributes").exists()
    }

    /// Resolve `HEAD^{tree}` to its oid. `None` on failure (unborn HEAD,
    /// corrupt repo) — callers treat that as "nothing proven".
    fn head_tree_oid(&self) -> Option<String> {
        let output = self
            .execute_git_command(&["rev-parse", "HEAD^{tree}"], "")
            .ok()?;
        let output = String::from_utf8(output).ok()?;
        let trimmed = output.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    fn finish_dirty_hash(&self, mut hasher: sha2::Sha256, has_status: bool) -> Option<String> {
        use sha2::Digest;

        // Try `git diff HEAD` first. In a freshly initialized repo with no
        // commits, HEAD doesn't exist and git exits with code 128. Fall back
        // to `git diff --cached` which diffs the index against an empty tree,
        // correctly capturing staged file content without needing HEAD.
        let diff_has_content = match self.stream_diff_into_hasher(
            &["diff", "HEAD", "--no-ext-diff", "--no-color"],
            &mut hasher,
        ) {
            Some(has_content) => has_content,
            None => match self.stream_diff_into_hasher(
                &["diff", "--cached", "--no-ext-diff", "--no-color"],
                &mut hasher,
            ) {
                Some(has_content) => has_content,
                None => {
                    turborepo_log::warn(
                        turborepo_log::Source::turbo(turborepo_log::Subsystem::Scm),
                        "failed to run git diff for dirty hash",
                    )
                    .emit();
                    false
                }
            },
        };

        if !has_status && !diff_has_content {
            return None;
        }

        Some(hex::encode(hasher.finalize()))
    }

    /// Spawn a git diff subprocess, streaming its stdout into `hasher`.
    /// Returns whether the diff had content, or `None` if the command failed.
    fn stream_diff_into_hasher(&self, args: &[&str], hasher: &mut sha2::Sha256) -> Option<bool> {
        use std::{io::Read, process::Stdio};

        use sha2::Digest;

        let mut child = match Command::new(self.bin.as_std_path())
            .args(args)
            .current_dir(&self.root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return None,
        };

        let mut has_content = false;
        if let Some(stdout) = child.stdout.take() {
            let mut reader = std::io::BufReader::new(stdout);
            let mut buf = [0u8; 65536];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        has_content = true;
                        hasher.update(&buf[..n]);
                    }
                    Err(_) => break,
                }
            }
        }

        child
            .wait()
            .is_ok_and(|s| s.success())
            .then_some(has_content)
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
        if let Ok(pr) = base_ref_env.github_base_ref
            && !pr.is_empty()
        {
            return Some(pr);
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
            Self::validate_git_ref(valid_from)?;
            return Ok(valid_from.to_string());
        }

        if let Some(github_base_ref) = Self::get_github_base_ref(env) {
            Self::validate_git_ref(&github_base_ref)?;
            // we don't fall through to checking against main or master
            // because at this point we know we're in a GITHUB CI environment
            // and we should really know by now what the base ref is
            // so it's better to just error if something went wrong
            return match self
                .execute_git_command(&["rev-parse", "--end-of-options", &github_base_ref], "")
            {
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

        default_base_ref(|branch| self.execute_git_command(&["rev-parse", branch], "").is_ok())
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
        Self::validate_git_ref(to_commit)?;
        let mut args = vec!["diff-tree", "-r", "--name-only", "--no-commit-id", "-z"];

        if merge_base {
            args.push("--merge-base");
        }

        args.extend(["--end-of-options", &valid_from, to_commit]);

        let output = self.execute_git_command(&args, pathspec)?;
        self.add_files_from_stdout(&mut files, turbo_root, output)?;

        // We only care about non-tracked files if we haven't specified both ends up the
        // comparison
        if include_uncommitted {
            // Add untracked files or unstaged changes, i.e. files that are not in git at
            // all
            let ls_files_output = self.execute_git_command(
                &[
                    "ls-files",
                    "--others",
                    "--modified",
                    "--exclude-standard",
                    "-z",
                ],
                pathspec,
            )?;
            self.add_files_from_stdout(&mut files, turbo_root, ls_files_output)?;
            // Include any files that have been staged, but not committed
            let diff_output =
                self.execute_git_command(&["diff", "--name-only", "--cached", "-z"], pathspec)?;
            self.add_files_from_stdout(&mut files, turbo_root, diff_output)?;
        }

        Ok(files)
    }

    pub fn execute_git_command(&self, args: &[&str], pathspec: &str) -> Result<Vec<u8>, Error> {
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
    ) -> Result<(), Error> {
        let stdout = String::from_utf8_lossy(&stdout);
        for line in stdout.split('\0') {
            if line.is_empty() {
                continue;
            }
            let path = RelativeUnixPath::new(line)?;
            let anchored_to_turbo_root_file_path =
                self.reanchor_path_from_git_root_to_turbo_root(turbo_root, path)?;
            files.insert(anchored_to_turbo_root_file_path);
        }
        Ok(())
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

        self.execute_git_command(&["show", "--end-of-options", &arg], "")
    }
}

fn default_base_ref(mut branch_exists: impl FnMut(&str) -> bool) -> Result<String, Error> {
    if branch_exists("main") {
        return Ok("main".to_string());
    }

    if branch_exists("master") {
        return Ok("master".to_string());
    }

    Err(Error::UnableToResolveRef)
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
        assert_matches,
        collections::HashSet,
        env::VarError,
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use tempfile::{NamedTempFile, TempDir};
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError};
    use which::which;

    use super::{CIEnv, InvalidRange, default_base_ref, previous_content};
    use crate::{
        Error, GitRepo, RepoGitIndex, SCM,
        git::{GitHubCommit, GitHubEvent},
    };

    fn run_git(repo_root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn setup_repository(initial_head: Option<&str>) -> Result<(TempDir, PathBuf), Error> {
        let repo_root = tempfile::tempdir()?;
        run_git(repo_root.path(), &["init"]);
        if let Some(branch) = initial_head {
            run_git(
                repo_root.path(),
                &["symbolic-ref", "HEAD", &format!("refs/heads/{}", branch)],
            );
        }
        run_git(repo_root.path(), &["config", "user.name", "test"]);
        run_git(
            repo_root.path(),
            &["config", "user.email", "test@example.com"],
        );

        let path = repo_root.path().to_path_buf();
        Ok((repo_root, path))
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

    fn commit_file(repo_root: &Path, path: &Path, _previous_commit: Option<&str>) -> String {
        run_git(repo_root, &["add", &path.to_string_lossy()]);
        run_git(repo_root, &["commit", "-m", "Commit"]);
        run_git(repo_root, &["rev-parse", "HEAD"])
    }

    fn commit_delete(repo_root: &Path, path: &Path) -> String {
        run_git(repo_root, &["rm", &path.to_string_lossy()]);
        run_git(repo_root, &["commit", "-m", "Commit"]);
        run_git(repo_root, &["rev-parse", "HEAD"])
    }

    fn commit_rename(repo_root: &Path, source: &Path, dest: &Path) -> String {
        run_git(repo_root, &["rm", &source.to_string_lossy()]);
        run_git(repo_root, &["add", &dest.to_string_lossy()]);
        run_git(repo_root, &["commit", "-m", "Commit"]);
        run_git(repo_root, &["rev-parse", "HEAD"])
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

        assert!(
            changed_files(
                tmp_dir.path().to_owned(),
                tmp_dir.path().to_owned(),
                Some("HEAD~1"),
                Some("HEAD"),
                false,
            )
            .is_ok()
        );

        assert!(
            changed_files(
                tmp_dir.path().to_owned(),
                tmp_dir.path().to_owned(),
                Some("HEAD"),
                None,
                true,
            )
            .is_ok()
        );

        Ok(())
    }

    #[test]
    fn test_deleted_files() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(None)?;

        let file = repo_root.path().join("foo.js");
        let file_path = Path::new("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_sha = commit_file(&repo_path, file_path, None);

        fs::remove_file(&file)?;
        let _second_commit_sha = commit_delete(&repo_path, file_path);

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
        let (repo_root, repo_path) = setup_repository(None)?;

        let file = repo_root.path().join("foo.js");
        let file_path = Path::new("foo.js");
        fs::write(&file, "let z = 0;")?;

        let first_commit_sha = commit_file(&repo_path, file_path, None);

        fs::rename(file, repo_root.path().join("bar.js")).unwrap();

        let new_file_path = Path::new("bar.js");
        let _second_commit_sha = commit_rename(&repo_path, file_path, new_file_path);

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
        let (repo_root, repo_path) = setup_repository(None)?;
        let first_file = repo_root.path().join("foo.js");
        fs::write(first_file, "let z = 0;")?;
        // Create a base commit. This will *not* be the merge base
        let first_commit_sha = commit_file(&repo_path, Path::new("foo.js"), None);

        let second_file = repo_root.path().join("bar.js");
        fs::write(second_file, "let y = 1;")?;
        // This commit will be the merge base
        let second_commit_sha =
            commit_file(&repo_path, Path::new("bar.js"), Some(&first_commit_sha));

        let third_file = repo_root.path().join("baz.js");
        fs::write(third_file, "let x = 2;")?;
        // Create a first commit off of merge base
        let third_commit_sha =
            commit_file(&repo_path, Path::new("baz.js"), Some(&second_commit_sha));

        // Move HEAD back to merge base without resetting the working tree.
        // `git reset --soft` moves HEAD but keeps the index and working tree intact,
        // matching the old git2 `set_head_detached` behavior.
        run_git(&repo_path, &["reset", "--soft", &second_commit_sha]);
        let fourth_file = repo_root.path().join("qux.js");
        fs::write(fourth_file, "let w = 3;")?;
        // Create a second commit off of merge base
        let fourth_commit_sha =
            commit_file(&repo_path, Path::new("qux.js"), Some(&second_commit_sha));

        run_git(&repo_path, &["checkout", "--detach", &third_commit_sha]);
        let merge_base = run_git(
            &repo_path,
            &["merge-base", &third_commit_sha, &fourth_commit_sha],
        );

        assert_eq!(merge_base, second_commit_sha);

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some(&third_commit_sha),
            Some(&fourth_commit_sha),
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
        let (repo_root, repo_path) = setup_repository(None)?;
        let turbo_root = repo_root.path();
        let file = repo_root.path().join("foo.js");
        fs::write(file, "let z = 0;")?;

        // First commit (we need a base commit to compare against)
        let first_commit_sha = commit_file(&repo_path, Path::new("foo.js"), None);

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
        run_git(&repo_path, &["add", "bar.js"]);

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
        let second_commit_sha =
            commit_file(&repo_path, Path::new("bar.js"), Some(&first_commit_sha));

        // Test that only second file is marked as changed when we check commit range
        let files = changed_files(
            repo_root.path().to_path_buf(),
            turbo_root.to_path_buf(),
            Some(first_commit_sha.as_str()),
            Some(second_commit_sha.as_str()),
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
            Some(first_commit_sha.as_str()),
            Some(second_commit_sha.as_str()),
            false,
        )?;
        assert_eq!(files, HashSet::from(["bar.js".to_string()]));

        // Since we are only specifying the first commit, the new file should show up
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some(second_commit_sha.as_str()),
            None,
            true,
        )?;
        assert_eq!(
            files,
            HashSet::from([format!("subdir{}baz.js", std::path::MAIN_SEPARATOR)])
        );

        // Commit the new file so it shows up in the changed files
        let third_commit_sha = commit_file(
            &repo_path,
            &Path::new("subdir").join("baz.js"),
            Some(&second_commit_sha),
        );

        // Test that `turbo_root` filters out files not in the specified directory
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some(first_commit_sha.as_str()),
            Some(third_commit_sha.as_str()),
            false,
        )?;
        assert_eq!(files, HashSet::from(["baz.js".to_string()]));

        Ok(())
    }

    #[test]
    fn test_changed_files_with_root_as_relative() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(None)?;
        let file = repo_root.path().join("foo.js");
        fs::write(file, "let z = 0;")?;

        // First commit (we need a base commit to compare against)
        commit_file(&repo_path, Path::new("foo.js"), None);

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
        let (repo_root, repo_path) = setup_repository(None)?;

        fs::create_dir(repo_root.path().join("subdir"))?;
        // Create additional nested directory to test that we return a system path
        // and not a normalized unix path
        fs::create_dir(repo_root.path().join("subdir").join("src"))?;

        let file = repo_root.path().join("subdir").join("foo.js");
        fs::write(file, "let z = 0;")?;
        let first_commit_sha = commit_file(&repo_path, Path::new("subdir/foo.js"), None);

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

        commit_file(
            &repo_path,
            Path::new("subdir/src/bar.js"),
            Some(&first_commit_sha),
        );

        let head_sha = run_git(&repo_path, &["rev-parse", "HEAD"]);

        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().join("subdir"),
            Some(first_commit_sha.as_str()),
            Some(head_sha.as_str()),
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
        let (repo_root, repo_path) = setup_repository(None)?;

        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let file = root.join_component("foo.js");
        file.create_with_contents("let z = 0;")?;

        let first_commit_sha = commit_file(&repo_path, Path::new("foo.js"), None);
        fs::write(&file, "let z = 1;")?;
        let second_commit_sha =
            commit_file(&repo_path, Path::new("foo.js"), Some(&first_commit_sha));

        let content = previous_content(
            repo_root.path().to_path_buf(),
            Some(first_commit_sha.as_str()),
            file.to_string(),
        )?;

        assert_eq!(content, b"let z = 0;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            Some(second_commit_sha.as_str()),
            file.to_string(),
        )?;
        assert_eq!(content, b"let z = 1;");

        let content = previous_content(
            repo_root.path().to_path_buf(),
            Some(second_commit_sha.as_str()),
            "foo.js".to_string(),
        )?;
        assert_eq!(content, b"let z = 1;");

        Ok(())
    }

    #[test]
    fn test_revparse() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(None)?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let file = root.join_component("foo.js");
        file.create_with_contents("let z = 0;")?;

        let first_commit_sha = commit_file(&repo_path, Path::new("foo.js"), None);
        fs::write(&file, "let z = 1;")?;
        let second_commit_sha =
            commit_file(&repo_path, Path::new("foo.js"), Some(&first_commit_sha));

        let revparsed_head = run_git(&repo_path, &["rev-parse", "HEAD"]);
        assert_eq!(revparsed_head, second_commit_sha);
        let revparsed_head_minus_1 = run_git(&repo_path, &["rev-parse", "HEAD~1"]);
        assert_eq!(revparsed_head_minus_1, first_commit_sha);

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
        let third_commit_sha =
            commit_file(&repo_path, Path::new("bar.js"), Some(&second_commit_sha));
        run_git(&repo_path, &["branch", "release-1", &third_commit_sha]);

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

    #[test]
    fn test_default_base_ref_resolution() {
        for (branches, expected) in [
            (vec!["main"], Some("main")),
            (vec!["master"], Some("master")),
            (vec!["ziltoid"], None),
            (vec!["ziltoid", "main"], Some("main")),
            (vec!["ziltoid", "master"], Some("master")),
            (vec!["ziltoid", "master", "main"], Some("main")),
        ] {
            let branches = HashSet::<&str>::from_iter(branches);
            let actual = default_base_ref(|branch| branches.contains(branch)).ok();

            assert_eq!(actual.as_deref(), expected);
        }
    }

    #[test]
    fn test_base_resolution_uses_override_before_default_branches() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(Some("main"))?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let file = root.join_component("todo.txt");
        file.create_with_contents("1. make async Rust good")?;
        commit_file(&repo_path, Path::new("todo.txt"), None);

        let git = GitRepo::find(&root).unwrap();
        let actual = git.resolve_base(Some("ziltoid"), CIEnv::none())?;

        assert_eq!(actual, "ziltoid");

        Ok(())
    }

    #[test]
    fn test_changed_files_rejects_option_like_refs() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(None)?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let file = root.join_component("todo.txt");
        file.create_with_contents("1. reject git option injection")?;
        commit_file(&repo_path, Path::new("todo.txt"), None);

        let target = NamedTempFile::new()?;
        fs::write(target.path(), "keep")?;
        let injected_ref = format!("--output={}", target.path().display());

        let scm = SCM::new(&root);
        let base_result =
            scm.changed_files(&root, Some(&injected_ref), Some("HEAD"), false, false, true);

        assert_matches!(
            base_result,
            Err(Error::InvalidGitRef(ref git_ref)) if git_ref == &injected_ref
        );
        assert_eq!(fs::read_to_string(target.path())?, "keep");

        let head_result =
            scm.changed_files(&root, Some("HEAD"), Some(&injected_ref), false, false, true);

        assert_matches!(
            head_result,
            Err(Error::InvalidGitRef(ref git_ref)) if git_ref == &injected_ref
        );
        assert_eq!(fs::read_to_string(target.path())?, "keep");

        Ok(())
    }

    #[test]
    fn test_previous_content_rejects_option_like_ref() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(None)?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let file = root.join_component("todo.txt");
        file.create_with_contents("1. reject git option injection")?;
        commit_file(&repo_path, Path::new("todo.txt"), None);

        let target = NamedTempFile::new()?;
        fs::write(target.path(), "keep")?;
        let injected_ref = format!("--output={}", target.path().display());
        let result = previous_content(
            repo_root.path().to_path_buf(),
            Some(&injected_ref),
            file.to_string(),
        );

        assert_matches!(
            result,
            Err(Error::InvalidGitRef(ref git_ref)) if git_ref == &injected_ref
        );
        assert_eq!(fs::read_to_string(target.path())?, "keep");

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

        let (repo_root, _repo_path) = setup_repository(None)?;
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
        let (repo_root, repo_path) = setup_repository(Some("my-main"))?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        // WARNING:
        // if you do not make a commit, git will show you that you have no branches.
        let file = root.join_component("todo.txt");
        file.create_with_contents("1. explain why async Rust is good")?;
        let _first_commit = commit_file(&repo_path, Path::new("todo.txt"), None);

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

    #[test]
    fn test_unicode_filenames_in_changed_files() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(None)?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        // Test various Unicode filenames that should be properly handled with -z flag
        let test_files = vec![
            "测试文件.txt",      // Chinese
            "テストファイル.js", // Japanese
            "файл.rs",           // Cyrillic
            "file with spaces.txt",
            "emoji_🚀.md",
            "café.ts",  // Latin with diacritics
            "ñoño.jsx", // Spanish with tildes
            "αβγ.py",   // Greek
        ];

        // Create initial commit with a base file
        let base_file = root.join_component("base.txt");
        base_file.create_with_contents("base content")?;
        let first_commit_sha = commit_file(&repo_path, Path::new("base.txt"), None);

        // Create and commit all Unicode files
        for filename in &test_files {
            let file_path = root.join_component(filename);
            file_path.create_with_contents(format!("content for {}", filename))?;
        }

        // Get changed files with uncommitted Unicode files
        let scm = SCM::new(&root);
        let files = scm
            .changed_files(&root, Some("HEAD"), None, true, false, false)?
            .unwrap();

        // Verify all Unicode files are detected in uncommitted changes
        for filename in &test_files {
            assert!(
                files.iter().any(|f| f.to_string().contains(filename)),
                "Failed to detect uncommitted Unicode file: {}",
                filename
            );
        }

        // Commit all Unicode files
        let mut last_commit = first_commit_sha.clone();
        for filename in &test_files {
            last_commit = commit_file(&repo_path, Path::new(filename), Some(&last_commit));
        }

        // Test committed Unicode files in range
        let files = changed_files(
            repo_root.path().to_path_buf(),
            repo_root.path().to_path_buf(),
            Some(first_commit_sha.as_str()),
            Some("HEAD"),
            false,
        )?;

        // Verify all Unicode files are detected in commit range
        for filename in &test_files {
            assert!(
                files.iter().any(|f| f.contains(filename)),
                "Failed to detect committed Unicode file: {}",
                filename
            );
        }

        // Test modification of Unicode files
        let modified_file = "测试文件.txt";
        let file_path = root.join_component(modified_file);
        file_path.create_with_contents("modified content")?;

        let files = scm
            .changed_files(&root, Some("HEAD"), None, true, false, false)?
            .unwrap();

        assert!(
            files.iter().any(|f| f.to_string().contains(modified_file)),
            "Failed to detect modified Unicode file: {}",
            modified_file
        );

        // Test deletion of Unicode files
        let delete_file = "emoji_🚀.md";
        let file_path = root.join_component(delete_file);
        std::fs::remove_file(file_path.as_std_path())?;

        let files = scm
            .changed_files(&root, Some("HEAD"), None, true, false, false)?
            .unwrap();

        assert!(
            files.iter().any(|f| f.to_string().contains(delete_file)),
            "Failed to detect deleted Unicode file: {}",
            delete_file
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

        let actual = GitRepo::get_github_base_ref(CIEnv {
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

    #[test]
    fn test_dirty_hash_clean_tree_returns_none() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        assert_eq!(scm.get_dirty_hash(), None);
    }

    #[test]
    fn test_dirty_hash_unstaged_changes() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(&file, "modified").unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        assert!(scm.get_dirty_hash().is_some());
    }

    #[test]
    fn test_dirty_hash_staged_changes() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(&file, "staged content").unwrap();
        run_git(repo_root.path(), &["add", "foo.txt"]);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        assert!(scm.get_dirty_hash().is_some());
    }

    #[test]
    fn test_dirty_hash_untracked_file() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(repo_root.path().join("untracked.txt"), "new file").unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        assert!(scm.get_dirty_hash().is_some());
    }

    #[test]
    fn test_dirty_hash_deterministic() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(&file, "dirty").unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let hash1 = scm.get_dirty_hash();
        let hash2 = scm.get_dirty_hash();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_dirty_hash_different_content_produces_different_hash() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(&file, "content A").unwrap();
        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let hash_a = scm.get_dirty_hash();

        fs::write(&file, "content B").unwrap();
        let hash_b = scm.get_dirty_hash();

        assert_ne!(
            hash_a, hash_b,
            "different content should produce different hashes"
        );
    }

    #[test]
    fn test_dirty_hash_manual_scm_returns_none() {
        assert_eq!(SCM::Manual.get_dirty_hash(), None);
    }

    /// Differential matrix: the repo-index dirty hash must agree with the
    /// `git status`-based dirty hash on whether the tree is dirty, across
    /// every class of working-tree state. This is the contract that protects
    /// cache provenance metadata from regressing.
    #[test]
    fn test_dirty_hash_paths_agree_across_states() {
        type Mutation = fn(&Path);
        let cases: &[(&str, Mutation, bool)] = &[
            ("clean", |_root| {}, false),
            (
                "unstaged_modification",
                |root| fs::write(root.join("foo.txt"), "modified").unwrap(),
                true,
            ),
            (
                "staged_modification",
                |root| {
                    fs::write(root.join("foo.txt"), "staged").unwrap();
                    run_git(root, &["add", "foo.txt"]);
                },
                true,
            ),
            (
                "staged_new_file",
                |root| {
                    fs::write(root.join("brand_new.txt"), "new").unwrap();
                    run_git(root, &["add", "brand_new.txt"]);
                },
                true,
            ),
            (
                "staged_deletion",
                |root| {
                    run_git(root, &["rm", "-q", "foo.txt"]);
                },
                true,
            ),
            (
                "unstaged_deletion",
                |root| fs::remove_file(root.join("foo.txt")).unwrap(),
                true,
            ),
            (
                "untracked_file",
                |root| fs::write(root.join("untracked.txt"), "hi").unwrap(),
                true,
            ),
            (
                "untracked_nested",
                |root| {
                    fs::create_dir_all(root.join("deep/dir")).unwrap();
                    fs::write(root.join("deep/dir/f.txt"), "hi").unwrap();
                },
                true,
            ),
            (
                "gitignored_untracked",
                |root| fs::write(root.join("ignored.tmp"), "hi").unwrap(),
                false,
            ),
            (
                "intent_to_add",
                |root| {
                    fs::write(root.join("ita.txt"), "ita").unwrap();
                    run_git(root, &["add", "-N", "ita.txt"]);
                },
                true,
            ),
            (
                "racy_rewrite_same_content",
                |root| {
                    // Rewrite identical content: mtime changes, content does
                    // not. The gix index conservatively classifies this as
                    // modified; the dirty hash must not.
                    let path = root.join("foo.txt");
                    let content = fs::read(&path).unwrap();
                    fs::write(&path, content).unwrap();
                },
                false,
            ),
        ];

        for (name, mutate, expected_dirty) in cases {
            let (repo_root, repo_path) = setup_repository(None).unwrap();
            fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
            fs::write(repo_root.path().join(".gitignore"), "*.tmp\n").unwrap();
            commit_file(&repo_path, Path::new("foo.txt"), None);
            run_git(repo_root.path(), &["add", ".gitignore"]);
            run_git(repo_root.path(), &["commit", "-q", "-m", "gitignore"]);

            mutate(repo_root.path());

            let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
            let scm = SCM::new(&git_root);
            let git = make_git_repo(&git_root);
            let repo_index = RepoGitIndex::new(&git).unwrap();

            let old = scm.get_dirty_hash();
            let new = scm.get_dirty_hash_from_repo_index(&repo_index);

            assert_eq!(
                old.is_some(),
                *expected_dirty,
                "{name}: status-based path expected dirty={expected_dirty}, got {old:?}"
            );
            assert_eq!(
                new.is_some(),
                *expected_dirty,
                "{name}: repo-index path expected dirty={expected_dirty}, got {new:?}"
            );

            // Determinism: rebuilding the index over the same state must
            // reproduce the same hash.
            let repo_index2 = RepoGitIndex::new(&git).unwrap();
            let new2 = scm.get_dirty_hash_from_repo_index(&repo_index2);
            assert_eq!(
                new, new2,
                "{name}: repo-index dirty hash must be deterministic"
            );
        }
    }

    #[test]
    fn test_dirty_hash_from_repo_index_unstaged_modification() {
        // Regression test: an unstaged modification to a tracked file must
        // produce a dirty hash. An earlier version of the repo-index path
        // short-circuited on `git diff --cached` (staged-only) and missed
        // this, the most common dirty state.
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(&file, "modified but not staged").unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(
            scm.get_dirty_hash_from_repo_index(&repo_index).is_some(),
            "unstaged modification must produce a dirty hash"
        );
    }

    #[test]
    fn test_dirty_hash_from_repo_index_untracked_name_sensitivity() {
        // Different untracked file names must produce different hashes.
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);

        fs::write(repo_root.path().join("name_one.txt"), "x").unwrap();
        let hash_one = scm.get_dirty_hash_from_repo_index(&RepoGitIndex::new(&git).unwrap());
        fs::remove_file(repo_root.path().join("name_one.txt")).unwrap();

        fs::write(repo_root.path().join("name_two.txt"), "x").unwrap();
        let hash_two = scm.get_dirty_hash_from_repo_index(&RepoGitIndex::new(&git).unwrap());

        assert_ne!(
            hash_one, hash_two,
            "different untracked file names must produce different hashes"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_dirty_hash_untracked_symlink_agreement() {
        // An untracked symlink is the one entry class where the repo-index
        // untracked walk (regular files only) and `git status` (lists
        // symlinks) could disagree. Pin whatever the truth is.
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        std::os::unix::fs::symlink("foo.txt", repo_root.path().join("untracked_link")).unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        let old = scm.get_dirty_hash();
        let new = scm.get_dirty_hash_from_repo_index(&repo_index);

        assert_eq!(
            old.is_some(),
            new.is_some(),
            "untracked symlink dirtiness must agree: status-based={old:?}, repo-index={new:?}"
        );
        assert!(new.is_some(), "untracked symlink must dirty the tree");

        // A broken symlink is still untracked state.
        fs::remove_file(repo_root.path().join("untracked_link")).unwrap();
        std::os::unix::fs::symlink("does_not_exist", repo_root.path().join("broken_link")).unwrap();
        let repo_index = RepoGitIndex::new(&git).unwrap();
        let broken = scm.get_dirty_hash_from_repo_index(&repo_index);
        assert!(
            broken.is_some(),
            "broken untracked symlink must dirty the tree"
        );
        assert_ne!(
            new, broken,
            "different symlink names must produce different hashes"
        );
    }

    #[test]
    fn test_dirty_hash_from_repo_index_clean_tree_returns_none() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert_eq!(scm.get_dirty_hash_from_repo_index(&repo_index), None);
    }

    #[test]
    fn test_dirty_hash_from_repo_index_untracked_file() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(repo_root.path().join("untracked.txt"), "new file").unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(scm.get_dirty_hash_from_repo_index(&repo_index).is_some());
    }

    #[test]
    fn test_dirty_hash_from_repo_index_staged_changes() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");
        fs::write(&file, "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        fs::write(&file, "staged content").unwrap();
        run_git(repo_root.path(), &["add", "foo.txt"]);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(scm.get_dirty_hash_from_repo_index(&repo_index).is_some());
    }

    /// After `git read-tree HEAD` every index entry loses its stat info, so
    /// every tracked file becomes a verification candidate (the
    /// snapshot-restored-workspace scenario). On a clean tree the
    /// verification pass must prove `git diff HEAD` empty and skip it.
    #[test]
    fn test_dirty_hash_stale_index_clean_tree_skips_diff() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);
        fs::write(repo_root.path().join("bar.txt"), "other content\n").unwrap();
        run_git(repo_root.path(), &["add", "bar.txt"]);
        run_git(repo_root.path(), &["commit", "-q", "-m", "bar"]);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("foo.txt", repo_root.path().join("link.txt")).unwrap();
            run_git(repo_root.path(), &["add", "link.txt"]);
            run_git(repo_root.path(), &["commit", "-q", "-m", "link"]);
        }

        run_git(repo_root.path(), &["read-tree", "HEAD"]);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(
            repo_index.tracked_diff_clean_root(false).is_some(),
            "clean stale index must prove the tree clean"
        );
        assert_eq!(scm.get_dirty_hash_from_repo_index(&repo_index), None);
        assert_eq!(scm.get_dirty_hash(), None, "subprocess path must agree");
    }

    /// A genuinely modified file on a stale index must block the skip and
    /// produce a dirty hash through the diff path.
    #[test]
    fn test_dirty_hash_stale_index_modified_file() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        run_git(repo_root.path(), &["read-tree", "HEAD"]);
        fs::write(repo_root.path().join("foo.txt"), "changed").unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(repo_index.tracked_diff_clean_root(false).is_none());
        assert!(scm.get_dirty_hash_from_repo_index(&repo_index).is_some());
        assert!(scm.get_dirty_hash().is_some(), "subprocess path must agree");
    }

    /// Staged changes invalidate the index's cache-tree, so the skip gate
    /// must not fire even when every candidate verifies clean.
    #[test]
    fn test_dirty_hash_stale_index_staged_change_blocks_skip() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        run_git(repo_root.path(), &["read-tree", "HEAD"]);
        fs::write(repo_root.path().join("foo.txt"), "staged").unwrap();
        run_git(repo_root.path(), &["add", "foo.txt"]);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(
            repo_index.tracked_diff_clean_root(false).is_none(),
            "staged change must block the diff skip"
        );
        assert!(scm.get_dirty_hash_from_repo_index(&repo_index).is_some());
        assert!(scm.get_dirty_hash().is_some(), "subprocess path must agree");
    }

    /// An exec-bit flip is content-identical but `git diff HEAD` reports it
    /// as a mode change. Verification must not let the skip fire.
    #[cfg(unix)]
    #[test]
    fn test_dirty_hash_stale_index_exec_bit_flip() {
        use std::os::unix::fs::PermissionsExt;

        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);

        run_git(repo_root.path(), &["read-tree", "HEAD"]);
        fs::set_permissions(
            repo_root.path().join("foo.txt"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(
            repo_index.tracked_diff_clean_root(false).is_none(),
            "exec-bit mismatch must block the diff skip"
        );
        assert!(
            scm.get_dirty_hash_from_repo_index(&repo_index).is_some(),
            "mode change must produce a dirty hash"
        );
        assert!(scm.get_dirty_hash().is_some(), "subprocess path must agree");
    }

    /// CRLF-containing content that raw-matches the index blocks the skip
    /// (git's checkin conversion could disagree under configs we don't
    /// read), but the diff then confirms cleanliness — both paths must
    /// still report clean.
    #[test]
    fn test_dirty_hash_stale_index_crlf_content_stays_clean() {
        let (repo_root, _repo_path) = setup_repository(None).unwrap();
        run_git(repo_root.path(), &["config", "core.autocrlf", "false"]);
        fs::write(repo_root.path().join("win.txt"), b"a\r\nb\r\n").unwrap();
        run_git(repo_root.path(), &["add", "win.txt"]);
        run_git(repo_root.path(), &["commit", "-q", "-m", "crlf"]);

        run_git(repo_root.path(), &["read-tree", "HEAD"]);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        let evidence = repo_index.tracked_diff_clean_root(false);
        assert!(
            matches!(
                evidence,
                Some((
                    _,
                    crate::repo_index::EolSensitivity::RequiresInertEolConversion
                ))
            ),
            "CRLF text content must make the proof eol-sensitive, got {evidence:?}"
        );
        // Whether the skip fires depends on the machine's git config; the
        // observable result must be clean either way (skip and diff agree).
        assert_eq!(
            scm.get_dirty_hash_from_repo_index(&repo_index),
            None,
            "tree must be reported clean"
        );
        assert_eq!(scm.get_dirty_hash(), None, "subprocess path must agree");
    }

    /// Binary content with CRLFs must not be treated as config-independent:
    /// a forced `text` attribute from a global/system attributes file makes
    /// git eol-convert even binary content, so the proof must stay
    /// eol-sensitive.
    #[test]
    fn test_dirty_hash_stale_index_binary_crlf_content_stays_eol_sensitive() {
        let (repo_root, _repo_path) = setup_repository(None).unwrap();
        run_git(repo_root.path(), &["config", "core.autocrlf", "false"]);
        fs::write(repo_root.path().join("blob.bin"), b"\x00binary\r\ndata\r\n").unwrap();
        run_git(repo_root.path(), &["add", "blob.bin"]);
        run_git(repo_root.path(), &["commit", "-q", "-m", "bin"]);

        run_git(repo_root.path(), &["read-tree", "HEAD"]);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        let evidence = repo_index.tracked_diff_clean_root(false);
        assert!(
            matches!(
                evidence,
                Some((
                    _,
                    crate::repo_index::EolSensitivity::RequiresInertEolConversion
                ))
            ),
            "binary CRLF content must make the proof eol-sensitive, got {evidence:?}"
        );
        // Whether the skip fires depends on the machine's git config; the
        // observable result must be clean either way (skip and diff agree).
        assert_eq!(
            scm.get_dirty_hash_from_repo_index(&repo_index),
            None,
            "tree must be reported clean"
        );
        assert_eq!(scm.get_dirty_hash(), None, "subprocess path must agree");
    }

    /// A retargeted symlink on a stale index must be caught as dirty.
    #[cfg(unix)]
    #[test]
    fn test_dirty_hash_stale_index_symlink_retarget() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);
        std::os::unix::fs::symlink("foo.txt", repo_root.path().join("link.txt")).unwrap();
        run_git(repo_root.path(), &["add", "link.txt"]);
        run_git(repo_root.path(), &["commit", "-q", "-m", "link"]);

        run_git(repo_root.path(), &["read-tree", "HEAD"]);
        fs::remove_file(repo_root.path().join("link.txt")).unwrap();
        std::os::unix::fs::symlink("elsewhere.txt", repo_root.path().join("link.txt")).unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let git = make_git_repo(&git_root);
        let repo_index = RepoGitIndex::new(&git).unwrap();

        assert!(repo_index.tracked_diff_clean_root(false).is_none());
        assert!(scm.get_dirty_hash_from_repo_index(&repo_index).is_some());
        assert!(scm.get_dirty_hash().is_some(), "subprocess path must agree");
    }

    /// Verified stat-stale entries must yield identical package hashes to a
    /// fresh index, with nothing left to re-hash per package.
    #[test]
    fn test_stale_index_package_hashes_match_fresh() {
        let (repo_root, repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("foo.txt"), "initial").unwrap();
        commit_file(&repo_path, Path::new("foo.txt"), None);
        fs::write(repo_root.path().join("bar.txt"), "more\n").unwrap();
        run_git(repo_root.path(), &["add", "bar.txt"]);
        run_git(repo_root.path(), &["commit", "-q", "-m", "bar"]);

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let git = make_git_repo(&git_root);
        let prefix = turbopath::RelativeUnixPathBuf::new("").unwrap();

        let fresh = RepoGitIndex::new(&git).unwrap();
        let (fresh_hashes, _fresh_to_hash) = fresh.get_package_hashes(&prefix).unwrap();

        run_git(repo_root.path(), &["read-tree", "HEAD"]);

        let stale = RepoGitIndex::new(&git).unwrap();
        let (stale_hashes, stale_to_hash) = stale.get_package_hashes(&prefix).unwrap();

        assert_eq!(
            fresh_hashes, stale_hashes,
            "stale-index package hashes must match fresh-index hashes"
        );
        assert!(
            stale_to_hash.is_empty(),
            "verified entries must not be deferred to per-package hashing, got {stale_to_hash:?}"
        );
    }

    #[test]
    fn test_dirty_hash_no_commits_untracked_file() {
        let (repo_root, _repo_path) = setup_repository(None).unwrap();
        fs::write(repo_root.path().join("new.txt"), "hello").unwrap();

        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        assert!(
            scm.get_dirty_hash().is_some(),
            "fresh repo with untracked files should produce a dirty hash"
        );
    }

    #[test]
    fn test_dirty_hash_no_commits_staged_content_affects_hash() {
        let (repo_root, _repo_path) = setup_repository(None).unwrap();
        let file = repo_root.path().join("foo.txt");

        fs::write(&file, "content A").unwrap();
        run_git(repo_root.path(), &["add", "foo.txt"]);
        let git_root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();
        let scm = SCM::new(&git_root);
        let hash_a = scm.get_dirty_hash();

        fs::write(&file, "content B").unwrap();
        run_git(repo_root.path(), &["add", "foo.txt"]);
        let hash_b = scm.get_dirty_hash();

        assert_ne!(
            hash_a, hash_b,
            "different staged content in a fresh repo should produce different hashes"
        );
    }

    fn make_git_repo(root: &AbsoluteSystemPath) -> GitRepo {
        let bin = GitRepo::find_bin().expect("git binary required for tests");
        GitRepo {
            root: root.to_owned(),
            bin,
            attrs: std::sync::OnceLock::new(),
            slowest_files: None,
        }
    }

    #[test]
    fn test_add_files_from_stdout_rejects_absolute_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let repo = make_git_repo(&root);

        let stdout = b"/absolute/path\0normal/file\0".to_vec();
        let mut files = HashSet::new();
        let result = repo.add_files_from_stdout(&mut files, &root, stdout);

        assert_matches!(result, Err(Error::Path(PathError::NotRelative(_), _)));
        assert!(
            files.is_empty(),
            "no files should be added before the error"
        );
    }

    #[test]
    fn test_add_files_from_stdout_rejects_unanchorable_paths() {
        let git_root_dir = tempfile::tempdir().unwrap();
        let turbo_root_dir = tempfile::tempdir().unwrap();
        let git_root = AbsoluteSystemPathBuf::try_from(git_root_dir.path()).unwrap();
        let turbo_root = AbsoluteSystemPathBuf::try_from(turbo_root_dir.path()).unwrap();
        let repo = make_git_repo(&git_root);

        // Path is valid and relative, but when joined with git_root it produces an
        // absolute path that isn't under turbo_root — reanchoring fails.
        let stdout = b"some/file.txt\0".to_vec();
        let mut files = HashSet::new();
        let result = repo.add_files_from_stdout(&mut files, &turbo_root, stdout);

        assert_matches!(result, Err(Error::Path(PathError::NotParent(_, _), _)));
    }

    #[test]
    fn test_changed_files_propagates_path_error() -> Result<(), Error> {
        let (repo_root, repo_path) = setup_repository(None)?;
        let root = AbsoluteSystemPathBuf::try_from(repo_root.path()).unwrap();

        let file = root.join_component("foo.txt");
        file.create_with_contents("content")?;
        commit_file(&repo_path, Path::new("foo.txt"), None);

        let new_file = root.join_component("bar.txt");
        new_file.create_with_contents("new content")?;

        // Use a turbo_root that is NOT a subdirectory of the git root.
        // The turbo_root_relative_to_git_root check at the start of changed_files
        // will fail, producing Error::Path.
        let separate_dir = tempfile::tempdir()?;
        let bad_turbo_root = AbsoluteSystemPathBuf::try_from(separate_dir.path()).unwrap();
        let scm = SCM::new(&root);

        let result = scm.changed_files(&bad_turbo_root, Some("HEAD"), None, true, false, false);
        assert_matches!(result, Err(Error::Path(PathError::NotParent(_, _), _)));

        Ok(())
    }
}
