#![cfg(feature = "git2")]
use std::str::FromStr;

use globwalk::ValidatedGlob;
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, PathError};
use turborepo_telemetry::events::task::{FileHashMethod, PackageTaskEventBuilder};

#[cfg(feature = "git2")]
use crate::hash_object::hash_objects;
use crate::{Error, GitHashes, GitRepo, RepoGitIndex, SCM};

impl SCM {
    pub fn get_hashes_for_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        files: &[impl AsRef<AnchoredSystemPath>],
        allow_missing: bool,
    ) -> Result<GitHashes, Error> {
        if allow_missing {
            self.hash_existing_of(turbo_root, files.iter())
        } else {
            self.hash_files(turbo_root, files.iter())
        }
    }

    #[tracing::instrument(skip(self, turbo_root, package_path, inputs, repo_index))]
    pub fn get_package_file_hashes<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        include_default_files: bool,
        telemetry: Option<PackageTaskEventBuilder>,
        repo_index: Option<&RepoGitIndex>,
    ) -> Result<GitHashes, Error> {
        match self {
            SCM::Manual => {
                if let Some(telemetry) = telemetry {
                    telemetry.track_file_hash_method(FileHashMethod::Manual);
                }
                crate::manual::get_package_file_hashes_without_git(
                    turbo_root,
                    package_path,
                    inputs,
                    include_default_files,
                )
            }
            SCM::Git(git) => {
                let result = git.get_package_file_hashes(
                    turbo_root,
                    package_path,
                    inputs,
                    include_default_files,
                    repo_index,
                );
                match result {
                    Ok(hashes) => {
                        if let Some(telemetry) = telemetry {
                            telemetry.track_file_hash_method(FileHashMethod::Git);
                        }
                        Ok(hashes)
                    }
                    Err(err) => {
                        // If the error is a resource exhaustion (e.g. too many
                        // open files), falling back to manual hashing will fail
                        // too. Propagate directly.
                        if err.is_resource_exhaustion() {
                            warn!(
                                "git hashing failed for {:?} with resource error: {}",
                                package_path, err,
                            );
                            return Err(err);
                        }

                        debug!(
                            "git hashing failed for {:?}: {}. Falling back to manual",
                            package_path, err,
                        );
                        if let Some(telemetry) = telemetry {
                            telemetry.track_file_hash_method(FileHashMethod::Manual);
                        }
                        crate::manual::get_package_file_hashes_without_git(
                            turbo_root,
                            package_path,
                            inputs,
                            include_default_files,
                        )
                    }
                }
            }
        }
    }

    pub fn hash_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        files: impl Iterator<Item = impl AsRef<AnchoredSystemPath>>,
    ) -> Result<GitHashes, Error> {
        match self {
            SCM::Manual => crate::manual::hash_files(turbo_root, files, false),
            SCM::Git(git) => git.hash_files(turbo_root, files),
        }
    }

    // hash_existing_of takes a list of files to hash and returns the hashes for the
    // files in that list that exist. Files in the list that do not exist are
    // skipped.
    pub fn hash_existing_of(
        &self,
        turbo_root: &AbsoluteSystemPath,
        files: impl Iterator<Item = impl AsRef<AnchoredSystemPath>>,
    ) -> Result<GitHashes, Error> {
        crate::manual::hash_files(turbo_root, files, true)
    }
}

impl GitRepo {
    fn get_package_file_hashes<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        include_default_files: bool,
        repo_index: Option<&RepoGitIndex>,
    ) -> Result<GitHashes, Error> {
        if inputs.is_empty() {
            return self.get_package_file_hashes_from_index(turbo_root, package_path, repo_index);
        }

        if !include_default_files {
            return self.get_package_file_hashes_from_inputs(
                turbo_root,
                package_path,
                inputs,
                true,
            );
        }

        self.get_package_file_hashes_from_inputs_and_index(
            turbo_root,
            package_path,
            inputs,
            repo_index,
        )
    }

    #[tracing::instrument(skip(self, turbo_root, repo_index))]
    fn get_package_file_hashes_from_index(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        repo_index: Option<&RepoGitIndex>,
    ) -> Result<GitHashes, Error> {
        let full_pkg_path = turbo_root.resolve(package_path);
        let git_to_pkg_path = self.root.anchor(&full_pkg_path)?;
        let pkg_prefix = git_to_pkg_path.to_unix();

        let (mut hashes, to_hash) = if let Some(index) = repo_index {
            index.get_package_hashes(&pkg_prefix)?
        } else {
            let mut hashes = self.git_ls_tree(&full_pkg_path)?;
            let to_hash = self.append_git_status(&full_pkg_path, &pkg_prefix, &mut hashes)?;
            (hashes, to_hash)
        };

        // Note: to_hash is *git repo relative*
        hash_objects(&self.root, &full_pkg_path, to_hash, &mut hashes)?;
        Ok(hashes)
    }

    fn hash_files(
        &self,
        process_relative_to: &AbsoluteSystemPath,
        files: impl Iterator<Item = impl AsRef<AnchoredSystemPath>>,
    ) -> Result<GitHashes, Error> {
        let mut hashes = GitHashes::new();
        let to_hash = files
            .map(|f| {
                Ok(self
                    .root
                    .anchor(process_relative_to.resolve(f.as_ref()))?
                    .to_unix())
            })
            .collect::<Result<Vec<_>, PathError>>()?;
        // Note: to_hash is *git repo relative*
        hash_objects(&self.root, process_relative_to, to_hash, &mut hashes)?;
        Ok(hashes)
    }

    #[tracing::instrument(skip(self, turbo_root, inputs))]
    fn get_package_file_hashes_from_inputs<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        include_configs: bool,
    ) -> Result<GitHashes, Error> {
        let full_pkg_path = turbo_root.resolve(package_path);
        let package_unix_path_buf = package_path.to_unix();
        let package_unix_path = package_unix_path_buf.as_str();

        static CONFIG_FILES: &[&str] = &["package.json", "turbo.json", "turbo.jsonc"];
        let extra_inputs = if include_configs { CONFIG_FILES } else { &[] };
        let total_inputs = inputs.len() + extra_inputs.len();

        let mut inclusions = Vec::with_capacity(total_inputs);
        let mut exclusions = Vec::with_capacity(total_inputs);
        let mut glob_buf = String::with_capacity(package_unix_path.len() + 1 + 64);

        let all_inputs = inputs
            .iter()
            .map(|s| s.as_ref())
            .chain(extra_inputs.iter().copied());
        for raw_glob in all_inputs {
            glob_buf.clear();
            if let Some(exclusion) = raw_glob.strip_prefix('!') {
                glob_buf.push_str(package_unix_path);
                glob_buf.push('/');
                glob_buf.push_str(exclusion.trim_start_matches('/'));
                exclusions.push(ValidatedGlob::from_str(&glob_buf)?);
            } else {
                glob_buf.push_str(package_unix_path);
                glob_buf.push('/');
                glob_buf.push_str(raw_glob.trim_start_matches('/'));
                inclusions.push(ValidatedGlob::from_str(&glob_buf)?);
            }
        }
        let files = globwalk::globwalk(
            turbo_root,
            &inclusions,
            &exclusions,
            globwalk::WalkType::Files,
        )?;
        let mut to_hash = Vec::with_capacity(files.len());
        for entry in &files {
            to_hash.push(self.root.anchor(entry)?.to_unix());
        }
        let mut hashes = GitHashes::with_capacity(files.len());
        hash_objects(&self.root, &full_pkg_path, to_hash, &mut hashes)?;
        Ok(hashes)
    }

    #[tracing::instrument(skip(self, turbo_root, inputs, repo_index))]
    fn get_package_file_hashes_from_inputs_and_index<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        repo_index: Option<&RepoGitIndex>,
    ) -> Result<GitHashes, Error> {
        // Start with ALL files from the git index (committed + dirty).
        let mut hashes =
            self.get_package_file_hashes_from_index(turbo_root, package_path, repo_index)?;

        let mut includes = Vec::new();
        let mut excludes = Vec::new();
        for input in inputs {
            let input_str = input.as_ref();
            if let Some(exclude) = input_str.strip_prefix('!') {
                excludes.push(exclude);
            } else {
                includes.push(input_str);
            }
        }

        // Include globs can find files not in the git index (e.g. gitignored files
        // that a user explicitly wants to track). Walk the filesystem for these
        // files but skip re-hashing any already known from the index.
        //
        // Optimization: separate literal file paths from actual glob patterns.
        // Literal paths (e.g. "$TURBO_ROOT$/tsconfig.json") are resolved with a
        // single stat syscall instead of compiling a glob regex and walking a
        // directory tree.
        let pkg_prefix = package_path.to_unix();

        if !includes.is_empty() {
            let full_pkg_path = turbo_root.resolve(package_path);
            let package_unix_path = pkg_prefix.as_str();

            static CONFIG_FILES: &[&str] = &["package.json", "turbo.json", "turbo.jsonc"];

            let mut glob_inclusions = Vec::new();
            let mut glob_exclusions = Vec::new();
            let mut literal_to_hash = Vec::new();
            let mut glob_buf = String::with_capacity(package_unix_path.len() + 1 + 64);

            let all = includes.iter().copied().chain(CONFIG_FILES.iter().copied());
            for raw_glob in all {
                glob_buf.clear();
                if let Some(exclusion) = raw_glob.strip_prefix('!') {
                    glob_buf.push_str(package_unix_path);
                    glob_buf.push('/');
                    glob_buf.push_str(exclusion.trim_start_matches('/'));
                    glob_exclusions.push(ValidatedGlob::from_str(&glob_buf)?);
                } else if !globwalk::is_glob_pattern(raw_glob) {
                    // Literal file path — resolve directly via stat instead of
                    // compiling a glob and walking directories.
                    let resolved =
                        full_pkg_path.join_unix_path(turbopath::RelativeUnixPath::new(raw_glob)?);
                    if resolved.symlink_metadata().is_ok() {
                        let git_relative = self.root.anchor(&resolved)?.to_unix();
                        let pkg_relative =
                            turbopath::RelativeUnixPath::strip_prefix(&git_relative, &pkg_prefix)
                                .ok()
                                .map(|s| s.to_owned());
                        let already_known = pkg_relative
                            .as_ref()
                            .is_some_and(|rel| hashes.contains_key(rel));
                        if !already_known {
                            literal_to_hash.push(git_relative);
                        }
                    }
                } else {
                    glob_buf.push_str(package_unix_path);
                    glob_buf.push('/');
                    glob_buf.push_str(raw_glob.trim_start_matches('/'));
                    glob_inclusions.push(ValidatedGlob::from_str(&glob_buf)?);
                }
            }

            // Hash any literal files discovered via direct stat.
            if !literal_to_hash.is_empty() {
                let mut new_hashes = GitHashes::with_capacity(literal_to_hash.len());
                hash_objects(&self.root, &full_pkg_path, literal_to_hash, &mut new_hashes)?;
                hashes.extend(new_hashes);
            }

            // Only do the expensive glob walk for patterns that are actual globs.
            if !glob_inclusions.is_empty() {
                let files = globwalk::globwalk(
                    turbo_root,
                    &glob_inclusions,
                    &glob_exclusions,
                    globwalk::WalkType::Files,
                )?;

                let mut to_hash = Vec::new();
                for entry in &files {
                    let git_relative = self.root.anchor(entry)?.to_unix();
                    let pkg_relative =
                        turbopath::RelativeUnixPath::strip_prefix(&git_relative, &pkg_prefix)
                            .ok()
                            .map(|s| s.to_owned());
                    let already_known = pkg_relative
                        .as_ref()
                        .is_some_and(|rel| hashes.contains_key(rel));
                    if !already_known {
                        to_hash.push(git_relative);
                    }
                }

                if !to_hash.is_empty() {
                    let mut new_hashes = GitHashes::with_capacity(to_hash.len());
                    hash_objects(&self.root, &full_pkg_path, to_hash, &mut new_hashes)?;
                    hashes.extend(new_hashes);
                }
            }
        }

        // Apply excludes via in-memory matching — no filesystem walk needed since
        // we already know all the paths from the combined index + includes.
        if !excludes.is_empty() {
            let exclude_globs: Vec<wax::Glob<'static>> = excludes
                .iter()
                .filter_map(|pattern| wax::Glob::new(pattern).ok().map(|g| g.into_owned()))
                .collect();

            if !exclude_globs.is_empty() {
                hashes.retain(|key, _| {
                    let path_str = key.as_str();
                    !exclude_globs
                        .iter()
                        .any(|glob| wax::Program::is_match(glob, path_str))
                });
            }
        }

        Ok(hashes)
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, collections::HashMap, process::Command};

    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf};

    use super::*;
    use crate::manual::get_package_file_hashes_without_git;

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
    }

    fn require_git_cmd(repo_root: &AbsoluteSystemPathBuf, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(repo_root);
        assert!(cmd.output().unwrap().status.success());
    }

    fn setup_repository(repo_root: &AbsoluteSystemPathBuf) {
        let cmds: &[&[&str]] = &[
            &["init", "."],
            &["config", "--local", "user.name", "test"],
            &["config", "--local", "user.email", "test@example.com"],
        ];
        for cmd in cmds {
            require_git_cmd(repo_root, cmd);
        }
    }

    fn commit_all(repo_root: &AbsoluteSystemPathBuf) {
        let cmds: &[&[&str]] = &[&["add", "."], &["commit", "-m", "foo"]];
        for cmd in cmds {
            require_git_cmd(repo_root, cmd);
        }
    }

    #[test]
    fn test_hash_symlink() {
        let (_, tmp_root) = tmp_dir();
        let git_root = tmp_root.join_component("actual_repo");
        git_root.create_dir_all().unwrap();
        setup_repository(&git_root);
        git_root.join_component("inside").create_dir_all().unwrap();
        let link = git_root.join_component("link");
        link.symlink_to_dir("inside").unwrap();
        let to_hash = vec![RelativeUnixPathBuf::new("link").unwrap()];
        let mut hashes = GitHashes::new();
        // FIXME: This test verifies a bug: we don't hash symlinks.
        // TODO: update this test to point at get_package_file_hashes
        hash_objects(&git_root, &git_root, to_hash, &mut hashes).unwrap();
        assert!(hashes.is_empty());

        let pkg_path = git_root.anchor(&git_root).unwrap();
        let manual_hashes =
            get_package_file_hashes_without_git(&git_root, &pkg_path, &["l*"], false).unwrap();
        assert!(manual_hashes.is_empty());
    }

    #[test]
    fn test_get_package_deps_fallback() {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let my_pkg_dir = repo_root.join_component("my-pkg");
        my_pkg_dir.create_dir_all().unwrap();

        // create file 1
        let committed_file_path = my_pkg_dir.join_component("committed-file");
        committed_file_path
            .create_with_contents("committed bytes")
            .unwrap();

        setup_repository(&repo_root);
        commit_all(&repo_root);
        let git = SCM::new(&repo_root);
        assert_matches!(git, SCM::Git(_));
        // Remove the .git directory to trigger an error in git hashing
        repo_root.join_component(".git").remove_dir_all().unwrap();
        let pkg_path = repo_root.anchor(&my_pkg_dir).unwrap();
        let hashes = git
            .get_package_file_hashes::<&str>(
                &repo_root,
                &pkg_path,
                &[],
                false,
                Some(PackageTaskEventBuilder::new("my-pkg", "test")),
                None,
            )
            .unwrap();
        let mut expected = GitHashes::new();
        expected.insert(
            RelativeUnixPathBuf::new("committed-file").unwrap(),
            "3a29e62ea9ba15c4a4009d1f605d391cdd262033".to_string(),
        );
        assert_eq!(hashes, expected);
    }

    #[test]
    fn test_get_package_deps() -> Result<(), Error> {
        // Directory structure:
        // <root>/
        //   package.json
        //   turbo.json
        //   .gitignore
        //   new-root-file <- new file not added to git
        //   my-pkg/
        //     package.json
        //     turbo.json
        //     $TURBO_DEFAULT$ <- ignored by git
        //     committed-file
        //     deleted-file
        //     uncommitted-file <- new file not added to git
        //     dir/
        //       nested-file
        //       ignored-file    <- ignored by git
        let (_repo_root_tmp, repo_root) = tmp_dir();

        // create a root package.json
        let root_pkg_json_path = repo_root.join_component("package.json");
        root_pkg_json_path.create_with_contents("{}")?;

        // create a root turbo.json
        let root_turbo_json_path = repo_root.join_component("turbo.json");
        root_turbo_json_path.create_with_contents("{}")?;

        // create the package directory
        let my_pkg_dir = repo_root.join_component("my-pkg");
        my_pkg_dir.create_dir_all()?;

        // create a gitignore file
        let gitignore_path = repo_root.join_component(".gitignore");
        gitignore_path.create_with_contents("my-pkg/dir/ignored-file\nmy-pkg/$TURBO_DEFAULT$")?;

        // create file 1
        let committed_file_path = my_pkg_dir.join_component("committed-file");
        committed_file_path.create_with_contents("committed bytes")?;

        // create file 2
        let deleted_file_path = my_pkg_dir.join_component("deleted-file");
        deleted_file_path.create_with_contents("delete-me")?;

        // create file 3
        let nested_file_path = my_pkg_dir.join_components(&["dir", "nested-file"]);
        nested_file_path.ensure_dir()?;
        nested_file_path.create_with_contents("nested")?;

        // create a package package.json
        let pkg_json_path = my_pkg_dir.join_component("package.json");
        pkg_json_path.create_with_contents("{}")?;

        // create a package turbo.json
        let turbo_json_path = my_pkg_dir.join_component("turbo.json");
        turbo_json_path.create_with_contents("{}")?;

        setup_repository(&repo_root);
        commit_all(&repo_root);
        let git = SCM::new(&repo_root);
        let SCM::Git(git) = git else {
            panic!("expected git, found {git:?}");
        };

        // remove a file
        deleted_file_path.remove()?;

        // create another untracked file in git
        let uncommitted_file_path = my_pkg_dir.join_component("uncommitted-file");
        uncommitted_file_path.create_with_contents("uncommitted bytes")?;

        // create an untracked file in git up a level
        let root_file_path = repo_root.join_component("new-root-file");
        root_file_path.create_with_contents("new-root bytes")?;

        // create a file that will be ignored
        let ignored_file_path = my_pkg_dir.join_components(&["dir", "ignored-file"]);
        ignored_file_path.ensure_dir()?;
        ignored_file_path.create_with_contents("ignored")?;

        // create a file that matches the $TURBO_DEFAULT$ token to test the edge case
        let token_file_path = my_pkg_dir.join_components(&["$TURBO_DEFAULT$"]);
        token_file_path.ensure_dir()?;
        token_file_path.create_with_contents("maybe-rename?")?;

        let package_path = AnchoredSystemPathBuf::from_raw("my-pkg")?;

        let all_expected = to_hash_map(&[
            ("committed-file", "3a29e62ea9ba15c4a4009d1f605d391cdd262033"),
            (
                "uncommitted-file",
                "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
            ),
            ("package.json", "9e26dfeeb6e641a33dae4961196235bdb965b21b"),
            ("turbo.json", "9e26dfeeb6e641a33dae4961196235bdb965b21b"),
            (
                "dir/nested-file",
                "bfe53d766e64d78f80050b73cd1c88095bc70abb",
            ),
        ]);
        let hashes =
            git.get_package_file_hashes::<&str>(&repo_root, &package_path, &[], false, None)?;
        assert_eq!(hashes, all_expected);

        // add the new root file as an option
        let mut all_expected = all_expected.clone();
        all_expected.insert(
            RelativeUnixPathBuf::new("../new-root-file").unwrap(),
            "8906ddcdd634706188bd8ef1c98ac07b9be3425e".to_string(),
        );
        all_expected.insert(
            RelativeUnixPathBuf::new("dir/ignored-file").unwrap(),
            "5537770d04ec8aaf7bae2d9ff78866de86df415c".to_string(),
        );
        all_expected.insert(
            RelativeUnixPathBuf::new("$TURBO_DEFAULT$").unwrap(),
            "2f26c7b914476b3c519e4f0fbc0d16c52a60d178".to_string(),
        );

        let input_tests: &[(&[&str], bool, &[&str])] = &[
            (
                &["uncommitted-file"],
                false,
                &["package.json", "turbo.json", "uncommitted-file"],
            ),
            (
                &["**/*-file"],
                false,
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                    "dir/nested-file",
                    "dir/ignored-file",
                ],
            ),
            (
                &["../**/*-file"],
                false,
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                    "dir/nested-file",
                    "dir/ignored-file",
                    "../new-root-file",
                ],
            ),
            (
                &["**/{uncommitted,committed}-file"],
                false,
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                ],
            ),
            (
                &["../**/{new-root,uncommitted,committed}-file"],
                false,
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                    "../new-root-file",
                ],
            ),
            (
                &["$TURBO_DEFAULT$"],
                true,
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                    "$TURBO_DEFAULT$",
                    "dir/nested-file",
                ],
            ),
            (
                &["$TURBO_DEFAULT$", "!dir/*"],
                true,
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                    "$TURBO_DEFAULT$",
                ],
            ),
            (
                &["$TURBO_DEFAULT$", "!committed-file", "dir/ignored-file"],
                true,
                &[
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                    "dir/ignored-file",
                    "dir/nested-file",
                    "$TURBO_DEFAULT$",
                ],
            ),
            (
                &["!committed-file", "$TURBO_DEFAULT$", "dir/ignored-file"],
                true,
                &[
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                    "dir/ignored-file",
                    "dir/nested-file",
                    "$TURBO_DEFAULT$",
                ],
            ),
        ];
        for (inputs, include_default_files, expected_files) in input_tests {
            let expected: GitHashes = HashMap::from_iter(expected_files.iter().map(|key| {
                let key = RelativeUnixPathBuf::new(*key).unwrap();
                let value = all_expected.get(&key).unwrap().clone();
                (key, value)
            }));

            let hashes = git
                .get_package_file_hashes(
                    &repo_root,
                    &package_path,
                    inputs,
                    *include_default_files,
                    None,
                )
                .unwrap();
            assert_eq!(hashes, expected);
        }
        Ok(())
    }

    fn to_hash_map(pairs: &[(&str, &str)]) -> GitHashes {
        HashMap::from_iter(
            pairs
                .iter()
                .map(|(path, hash)| (RelativeUnixPathBuf::new(*path).unwrap(), hash.to_string())),
        )
    }
}
