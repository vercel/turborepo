use std::str::FromStr;

use globwalk::ValidatedGlob;
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf, PathError};
use turborepo_telemetry::events::task::{FileHashMethod, PackageTaskEventBuilder};

use crate::{Error, GitHashes, GitRepo, RepoGitIndex, SCM, hash_object::hash_objects};

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
                    None,
                    None,
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
                            Some(&git.root),
                            git.git_attrs(),
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
            SCM::Manual => crate::manual::hash_files(turbo_root, files, false, None, None),
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
        let (attrs_root, cached_attrs) = match self {
            SCM::Git(git) => (Some(git.root.as_ref()), git.git_attrs()),
            SCM::Manual => (None, None),
        };
        crate::manual::hash_files(turbo_root, files, true, attrs_root, cached_attrs)
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
                repo_index,
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
        hash_objects(
            &self.root,
            &full_pkg_path,
            to_hash,
            &mut hashes,
            self.git_attrs(),
        )?;
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
        hash_objects(
            &self.root,
            process_relative_to,
            to_hash,
            &mut hashes,
            self.git_attrs(),
        )?;
        Ok(hashes)
    }

    fn git_relative_to_package_relative(
        &self,
        full_pkg_path: &AbsoluteSystemPath,
        pkg_prefix: &turbopath::RelativeUnixPathBuf,
        git_relative: &turbopath::RelativeUnixPathBuf,
    ) -> turbopath::RelativeUnixPathBuf {
        turbopath::RelativeUnixPath::strip_prefix(git_relative, pkg_prefix)
            .ok()
            .map(|stripped| stripped.to_owned())
            .unwrap_or_else(|| {
                let full_file_path = self.root.join_unix_path(git_relative);
                AnchoredSystemPathBuf::relative_path_between(full_pkg_path, &full_file_path)
                    .to_unix()
            })
    }

    fn extend_hashes_from_candidates(
        &self,
        full_pkg_path: &AbsoluteSystemPath,
        pkg_prefix: &turbopath::RelativeUnixPathBuf,
        repo_index: Option<&RepoGitIndex>,
        candidate_paths: Vec<turbopath::RelativeUnixPathBuf>,
        hashes: &mut GitHashes,
    ) -> Result<(), Error> {
        if candidate_paths.is_empty() {
            return Ok(());
        }

        let (known_hashes, to_hash) = if let Some(index) = repo_index {
            index.partition_existing_paths_for_hashing(candidate_paths)
        } else {
            (Vec::new(), candidate_paths)
        };

        hashes.reserve(known_hashes.len() + to_hash.len());
        for (git_relative, oid) in known_hashes {
            let package_relative =
                self.git_relative_to_package_relative(full_pkg_path, pkg_prefix, &git_relative);
            hashes.insert(package_relative, oid);
        }

        if !to_hash.is_empty() {
            let mut new_hashes = GitHashes::with_capacity(to_hash.len());
            hash_objects(
                &self.root,
                full_pkg_path,
                to_hash,
                &mut new_hashes,
                self.git_attrs(),
            )?;
            hashes.extend(new_hashes);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, turbo_root, inputs, repo_index))]
    fn get_package_file_hashes_from_inputs<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        include_configs: bool,
        repo_index: Option<&RepoGitIndex>,
    ) -> Result<GitHashes, Error> {
        let full_pkg_path = turbo_root.resolve(package_path);
        let package_unix_path_buf = package_path.to_unix();
        let pkg_prefix = self.root.anchor(&full_pkg_path)?.to_unix();
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
        let mut candidate_paths = Vec::with_capacity(files.len());
        for entry in &files {
            candidate_paths.push(self.root.anchor(entry)?.to_unix());
        }
        let mut hashes = GitHashes::with_capacity(files.len());
        self.extend_hashes_from_candidates(
            &full_pkg_path,
            &pkg_prefix,
            repo_index,
            candidate_paths,
            &mut hashes,
        )?;
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
                    match resolved.symlink_metadata() {
                        Ok(meta) if meta.is_dir() => {
                            // Directory literal — fall through to the glob
                            // walker which will expand it via
                            // add_doublestar_to_dir (e.g. "src" -> "src/**").
                            glob_buf.push_str(package_unix_path);
                            glob_buf.push('/');
                            glob_buf.push_str(raw_glob.trim_start_matches('/'));
                            glob_inclusions.push(ValidatedGlob::from_str(&glob_buf)?);
                        }
                        Ok(_) => {
                            let git_relative = self.root.anchor(&resolved)?.to_unix();
                            let pkg_relative = turbopath::RelativeUnixPath::strip_prefix(
                                &git_relative,
                                &pkg_prefix,
                            )
                            .ok()
                            .map(|s| s.to_owned());
                            let already_known = pkg_relative
                                .as_ref()
                                .is_some_and(|rel| hashes.contains_key(rel));
                            if !already_known {
                                literal_to_hash.push(git_relative);
                            }
                        }
                        Err(_) => {}
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
                self.extend_hashes_from_candidates(
                    &full_pkg_path,
                    &pkg_prefix,
                    repo_index,
                    literal_to_hash,
                    &mut hashes,
                )?;
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
                    self.extend_hashes_from_candidates(
                        &full_pkg_path,
                        &pkg_prefix,
                        repo_index,
                        to_hash,
                        &mut hashes,
                    )?;
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
    use std::{assert_matches, collections::HashMap, process::Command};

    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf};

    use super::*;
    use crate::{OidHash, manual::get_package_file_hashes_without_git};

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
        hash_objects(&git_root, &git_root, to_hash, &mut hashes, None).unwrap();
        assert!(hashes.is_empty());

        let pkg_path = git_root.anchor(&git_root).unwrap();
        let manual_hashes =
            get_package_file_hashes_without_git(&git_root, &pkg_path, &["l*"], false, None, None)
                .unwrap();
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
            OidHash::from_hex_str("3a29e62ea9ba15c4a4009d1f605d391cdd262033"),
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
            OidHash::from_hex_str("8906ddcdd634706188bd8ef1c98ac07b9be3425e"),
        );
        all_expected.insert(
            RelativeUnixPathBuf::new("dir/ignored-file").unwrap(),
            OidHash::from_hex_str("5537770d04ec8aaf7bae2d9ff78866de86df415c"),
        );
        all_expected.insert(
            RelativeUnixPathBuf::new("$TURBO_DEFAULT$").unwrap(),
            OidHash::from_hex_str("2f26c7b914476b3c519e4f0fbc0d16c52a60d178"),
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
                let value = *all_expected.get(&key).unwrap();
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

    /// Regression test for worktrees that live outside the main repo directory.
    ///
    /// Reproduces the real-world layout where:
    ///   ~/project/front           <- main repo
    ///   ~/project/front-worktree/ <- linked worktrees (sibling, NOT a child)
    ///
    /// Before the fix, `git_root` was set to the main worktree root, causing
    /// `self.root.anchor(turbo_root)` to fail with "Path X is not parent of Y"
    /// because the worktree path cannot be strip-prefixed by a sibling path.
    #[test]
    fn test_package_hashes_in_external_worktree() -> Result<(), Error> {
        use crate::worktree::WorktreeInfo;

        // Two separate temp dirs to simulate sibling directories
        let (_tmp_main, main_root) = tmp_dir();
        let (_tmp_wt, worktree_parent) = tmp_dir();

        // Set up the main repo with a package
        let pkg_dir = main_root.join_component("my-pkg");
        pkg_dir.create_dir_all()?;
        main_root
            .join_component("package.json")
            .create_with_contents("{}")?;
        pkg_dir
            .join_component("package.json")
            .create_with_contents("{}")?;
        pkg_dir
            .join_component("index.js")
            .create_with_contents("console.log('hello')")?;

        setup_repository(&main_root);
        commit_all(&main_root);

        // Create a linked worktree at a sibling path (not inside main_root)
        let worktree_path = worktree_parent.join_component("my-branch");
        require_git_cmd(
            &main_root,
            &[
                "worktree",
                "add",
                worktree_path.as_str(),
                "-b",
                "test-external-worktree",
            ],
        );

        // Detect worktree info from within the linked worktree
        let info = WorktreeInfo::detect(&worktree_path).unwrap();
        assert!(info.is_linked_worktree());
        assert_eq!(info.git_root, worktree_path);

        // Construct SCM the same way the run builder does: using the pre-resolved
        // git_root from worktree detection
        let scm = crate::SCM::new_with_git_root(&worktree_path, info.git_root);
        let crate::SCM::Git(git) = scm else {
            panic!("expected git SCM");
        };

        // This is the call that previously failed with "is not parent of"
        let package_path = AnchoredSystemPathBuf::from_raw("my-pkg")?;
        let hashes =
            git.get_package_file_hashes::<&str>(&worktree_path, &package_path, &[], false, None)?;

        assert!(
            hashes.contains_key(&RelativeUnixPathBuf::new("index.js").unwrap()),
            "should hash files in the worktree package"
        );
        assert!(
            hashes.contains_key(&RelativeUnixPathBuf::new("package.json").unwrap()),
            "should hash package.json in the worktree package"
        );

        Ok(())
    }

    #[test]
    fn test_inputs_in_nested_turbo_root() -> Result<(), Error> {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let turbo_root = repo_root.join_component("subdir");
        let my_pkg_dir = turbo_root.join_component("my-pkg");
        my_pkg_dir.create_dir_all()?;

        my_pkg_dir
            .join_component("committed-file")
            .create_with_contents("committed bytes")?;
        my_pkg_dir
            .join_component("package.json")
            .create_with_contents("{}")?;

        setup_repository(&repo_root);
        commit_all(&repo_root);

        my_pkg_dir
            .join_component("uncommitted-file")
            .create_with_contents("uncommitted bytes")?;

        let scm = crate::SCM::new_with_git_root(&turbo_root, repo_root.clone());
        let crate::SCM::Git(git) = scm else {
            panic!("expected git SCM");
        };
        let package_path = AnchoredSystemPathBuf::from_raw("my-pkg")?;

        let hashes =
            git.get_package_file_hashes(&turbo_root, &package_path, &["**/*-file"], false, None)?;

        let expected = to_hash_map(&[
            ("committed-file", "3a29e62ea9ba15c4a4009d1f605d391cdd262033"),
            (
                "uncommitted-file",
                "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
            ),
            ("package.json", "9e26dfeeb6e641a33dae4961196235bdb965b21b"),
        ]);

        assert_eq!(hashes, expected);
        Ok(())
    }

    fn to_hash_map(pairs: &[(&str, &str)]) -> GitHashes {
        HashMap::from_iter(pairs.iter().map(|(path, hash)| {
            (
                RelativeUnixPathBuf::new(*path).unwrap(),
                OidHash::from_hex_str(hash),
            )
        }))
    }

    /// Regression test: for LF-only files, the git path and manual path must
    /// produce identical hashes. With `text=auto` in `.gitattributes`, the
    /// normalization scan runs but should produce the same result as raw
    /// hashing since there are no CRLF pairs.
    #[test]
    fn test_git_and_manual_paths_agree_for_lf_files() {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let pkg_dir = repo_root.join_component("my-pkg");
        pkg_dir.create_dir_all().unwrap();

        // Add .gitattributes to exercise the normalization decision path
        std::fs::write(
            repo_root.as_std_path().join(".gitattributes"),
            "* text=auto\n",
        )
        .unwrap();

        let files: &[(&str, &[u8])] = &[
            ("my-pkg/package.json", b"{\"name\": \"my-pkg\"}\n"),
            ("my-pkg/index.ts", b"export const x = 1;\n"),
            ("my-pkg/data.json", b"{\"key\": \"value\"}\n"),
            (
                "my-pkg/binary.bin",
                &[0x89, 0x50, 0x4E, 0x47, 0x00, 0x01, 0x02],
            ),
        ];

        for (path, content) in files {
            let file_path =
                repo_root.join_unix_path(turbopath::RelativeUnixPath::new(path).unwrap());
            file_path.ensure_dir().unwrap();
            std::fs::write(file_path.as_std_path(), content).unwrap();
        }

        setup_repository(&repo_root);
        require_git_cmd(&repo_root, &["config", "--local", "core.autocrlf", "false"]);
        commit_all(&repo_root);

        let pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();

        // Hash via git path
        let git = SCM::new(&repo_root);
        let git_hashes = git
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[], false, None, None)
            .unwrap();

        // Hash via manual path
        let manual_hashes = get_package_file_hashes_without_git::<&str>(
            &repo_root,
            &pkg_path,
            &[],
            false,
            None,
            None,
        )
        .unwrap();

        // Filter to the same set of keys (manual path may pick up .gitignore etc.)
        for (path, git_hash) in &git_hashes {
            let manual_hash = manual_hashes.get(path);
            assert_eq!(
                Some(git_hash),
                manual_hash,
                "hash mismatch for {path}: git={git_hash}, manual={manual_hash:?}"
            );
        }
    }

    /// Regression test: binary files must never be normalized, even when
    /// .gitattributes says `text=auto`. The NUL-byte heuristic must detect
    /// them and hash raw bytes.
    #[test]
    fn test_binary_files_hash_raw_with_text_auto() {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let pkg_dir = repo_root.join_component("my-pkg");
        pkg_dir.create_dir_all().unwrap();

        // .gitattributes with text=auto
        std::fs::write(
            repo_root.as_std_path().join(".gitattributes"),
            "* text=auto\n",
        )
        .unwrap();

        // Binary files with CRLF bytes that must NOT be normalized
        let binary_cases: &[(&str, &[u8])] = &[
            // PNG-like header with NUL byte + CRLF
            (
                "my-pkg/image.png",
                &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00],
            ),
            // Arbitrary binary with NUL + CRLF in the middle
            ("my-pkg/data.bin", &[0xFF, 0x00, 0x0D, 0x0A, 0xFE, 0xFD]),
        ];

        std::fs::write(
            pkg_dir.as_std_path().join("package.json"),
            "{\"name\": \"my-pkg\"}\n",
        )
        .unwrap();
        for (path, content) in binary_cases {
            let full_path = repo_root.as_std_path().join(path);
            std::fs::create_dir_all(full_path.parent().unwrap()).unwrap();
            std::fs::write(&full_path, content).unwrap();
        }

        setup_repository(&repo_root);
        // Disable autocrlf to ensure git stores raw bytes
        require_git_cmd(&repo_root, &["config", "--local", "core.autocrlf", "false"]);
        commit_all(&repo_root);

        let pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();

        // Hash via git path
        let git = SCM::new(&repo_root);
        let git_hashes = git
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[], false, None, None)
            .unwrap();

        // Hash via manual path (no .git/) to verify both paths agree
        let (_prune_tmp, prune_root) = tmp_dir();
        let prune_pkg = prune_root.join_component("my-pkg");
        prune_pkg.create_dir_all().unwrap();
        std::fs::copy(
            repo_root.as_std_path().join(".gitattributes"),
            prune_root.as_std_path().join(".gitattributes"),
        )
        .unwrap();
        std::fs::copy(
            pkg_dir.as_std_path().join("package.json"),
            prune_pkg.as_std_path().join("package.json"),
        )
        .unwrap();
        for (rel_path, content) in binary_cases {
            let pkg_relative = rel_path.strip_prefix("my-pkg/").unwrap();
            std::fs::write(prune_pkg.as_std_path().join(pkg_relative), content).unwrap();
        }

        let prune_pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();
        let manual_hashes = get_package_file_hashes_without_git::<&str>(
            &prune_root,
            &prune_pkg_path,
            &[],
            false,
            None,
            None,
        )
        .unwrap();

        // Verify git and manual paths agree, and both match `git hash-object`
        for (rel_path, _content) in binary_cases {
            let pkg_relative = rel_path.strip_prefix("my-pkg/").unwrap();
            let full_path = repo_root.as_std_path().join(rel_path);

            let output = Command::new("git")
                .args(["hash-object", "--no-filters", full_path.to_str().unwrap()])
                .current_dir(repo_root.as_std_path())
                .output()
                .unwrap();
            let expected_hash = String::from_utf8(output.stdout).unwrap();
            let expected_hash = expected_hash.trim();

            let key = RelativeUnixPathBuf::new(pkg_relative).unwrap();
            let git_actual = git_hashes
                .get(&key)
                .expect("binary file should be in git hashes");
            assert_eq!(
                &**git_actual, expected_hash,
                "binary file {rel_path} should be hashed raw (no normalization) via git path"
            );

            let manual_actual = manual_hashes
                .get(&key)
                .expect("binary file should be in manual hashes");
            assert_eq!(
                &**manual_actual, expected_hash,
                "binary file {rel_path} should be hashed raw (no normalization) via manual path"
            );
        }
    }

    /// Bug reproduction for https://github.com/vercel/turborepo/issues/9616
    ///
    /// When .gitattributes has `text=auto`, git normalizes CRLF→LF in blobs.
    /// After `turbo prune`, the pruned output has no .git/ directory, so turbo
    /// falls back to manual hashing which hashes raw bytes (with CRLF). This
    /// produces a different hash than the git path.
    #[test]
    fn test_crlf_hash_matches_after_simulated_prune() {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let pkg_dir = repo_root.join_component("my-pkg");
        pkg_dir.create_dir_all().unwrap();

        // .gitattributes with text=auto (the common setup)
        std::fs::write(
            repo_root.as_std_path().join(".gitattributes"),
            "* text=auto\n",
        )
        .unwrap();

        let test_files: &[(&str, &[u8])] = &[
            // Text file with CRLF (the primary case)
            ("readme.md", b"Hello\r\nWorld\r\n"),
            // LF-only text file (should match trivially)
            ("lf-only.txt", b"line1\nline2\n"),
            // Binary file with CRLF bytes + NUL (must NOT be normalized)
            ("image.bin", &[0x89, 0x50, 0x0D, 0x0A, 0x00, 0x01]),
            // Mixed CRLF/LF
            ("mixed.txt", b"first\nsecond\r\nthird\n"),
        ];

        std::fs::write(
            pkg_dir.as_std_path().join("package.json"),
            "{\"name\": \"my-pkg\"}\n",
        )
        .unwrap();
        for (name, content) in test_files {
            std::fs::write(pkg_dir.as_std_path().join(name), content).unwrap();
        }

        setup_repository(&repo_root);
        require_git_cmd(&repo_root, &["config", "--local", "core.autocrlf", "false"]);
        commit_all(&repo_root);

        let pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();

        // Hash via git path (uses git ls-tree OIDs which are LF-normalized)
        let git = SCM::new(&repo_root);
        let git_hashes = git
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[], false, None, None)
            .unwrap();

        // Simulate turbo prune: copy files to a non-git directory
        let (_prune_tmp, prune_root) = tmp_dir();
        let prune_pkg = prune_root.join_component("my-pkg");
        prune_pkg.create_dir_all().unwrap();

        // Copy .gitattributes (our fix adds this to ADDITIONAL_FILES)
        std::fs::copy(
            repo_root.as_std_path().join(".gitattributes"),
            prune_root.as_std_path().join(".gitattributes"),
        )
        .unwrap();
        // Copy all package files byte-for-byte (like turbo prune does)
        std::fs::copy(
            pkg_dir.as_std_path().join("package.json"),
            prune_pkg.as_std_path().join("package.json"),
        )
        .unwrap();
        for (name, _) in test_files {
            std::fs::copy(
                pkg_dir.as_std_path().join(name),
                prune_pkg.as_std_path().join(name),
            )
            .unwrap();
        }

        let prune_pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();

        // Hash via manual path (no .git/ in pruned output)
        let manual_hashes = get_package_file_hashes_without_git::<&str>(
            &prune_root,
            &prune_pkg_path,
            &[],
            false,
            None,
            None,
        )
        .unwrap();

        for (name, _) in test_files {
            let key = RelativeUnixPathBuf::new(*name).unwrap();
            let git_hash = git_hashes
                .get(&key)
                .unwrap_or_else(|| panic!("{name} in git hashes"));
            let manual_hash = manual_hashes
                .get(&key)
                .unwrap_or_else(|| panic!("{name} in manual hashes"));

            assert_eq!(
                git_hash, manual_hash,
                "{name}: hash should match between git path and manual path (simulating turbo \
                 prune). git={git_hash}, manual={manual_hash}"
            );
        }
    }

    /// Bug reproduction for https://github.com/vercel/turborepo/issues/5081
    ///
    /// When a file with `text=auto` in .gitattributes is committed (git
    /// normalizes CRLF→LF in the blob), then touched (making it dirty but
    /// content-identical), the dirty-file hash should match the committed
    /// blob hash. Currently it doesn't because hash_objects() hashes raw
    /// bytes without applying .gitattributes filters.
    #[test]
    fn test_dirty_crlf_file_matches_committed_hash() {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let pkg_dir = repo_root.join_component("my-pkg");
        pkg_dir.create_dir_all().unwrap();

        std::fs::write(
            repo_root.as_std_path().join(".gitattributes"),
            "* text=auto\n",
        )
        .unwrap();

        std::fs::write(
            pkg_dir.as_std_path().join("readme.md"),
            b"Hello\r\nWorld\r\n",
        )
        .unwrap();
        std::fs::write(
            pkg_dir.as_std_path().join("package.json"),
            "{\"name\": \"my-pkg\"}\n",
        )
        .unwrap();

        setup_repository(&repo_root);
        require_git_cmd(&repo_root, &["config", "--local", "core.autocrlf", "false"]);
        commit_all(&repo_root);

        let pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();

        // Get committed hash (from git ls-tree — uses normalized blob OID)
        let git = SCM::new(&repo_root);
        let committed_hashes = git
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[], false, None, None)
            .unwrap();

        // Touch the file to make it dirty (content unchanged, mtime updated)
        let readme_path = pkg_dir.join_component("readme.md");
        let content = std::fs::read(readme_path.as_std_path()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::write(readme_path.as_std_path(), &content).unwrap();

        // Re-hash — now readme.md is dirty, goes through hash_objects()
        let dirty_hashes = git
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[], false, None, None)
            .unwrap();

        let key = RelativeUnixPathBuf::new("readme.md").unwrap();
        let committed = committed_hashes.get(&key).expect("readme.md committed");
        let dirty = dirty_hashes.get(&key).expect("readme.md dirty");

        assert_eq!(
            committed, dirty,
            "dirty CRLF file hash should match committed hash when content is identical. \
             committed={committed}, dirty={dirty}"
        );
    }

    /// Verify that package-scoped .gitattributes patterns resolve correctly
    /// through both git and manual hashing paths.
    ///
    /// Regression test for path relativity mismatch: hash_objects() uses
    /// git-root-relative paths while the manual path previously used
    /// package-relative paths for attribute resolution. A pattern like
    /// `my-pkg/*.md text` must match in both paths.
    #[test]
    fn test_package_scoped_gitattributes_pattern() {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let pkg_dir = repo_root.join_component("my-pkg");
        pkg_dir.create_dir_all().unwrap();

        // Pattern that references the package directory explicitly.
        // If the manual path passes only "readme.md" (package-relative) instead
        // of "my-pkg/readme.md" (root-relative), this pattern won't match.
        std::fs::write(
            repo_root.as_std_path().join(".gitattributes"),
            "my-pkg/*.md text\n",
        )
        .unwrap();

        std::fs::write(
            pkg_dir.as_std_path().join("readme.md"),
            b"Hello\r\nWorld\r\n",
        )
        .unwrap();
        std::fs::write(
            pkg_dir.as_std_path().join("package.json"),
            "{\"name\": \"my-pkg\"}\n",
        )
        .unwrap();

        setup_repository(&repo_root);
        require_git_cmd(&repo_root, &["config", "--local", "core.autocrlf", "false"]);
        commit_all(&repo_root);

        let pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();

        // Hash via git path
        let git = SCM::new(&repo_root);
        let git_hashes = git
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[], false, None, None)
            .unwrap();

        // Simulate prune: copy to a non-git directory
        let (_prune_tmp, prune_root) = tmp_dir();
        let prune_pkg = prune_root.join_component("my-pkg");
        prune_pkg.create_dir_all().unwrap();
        std::fs::copy(
            repo_root.as_std_path().join(".gitattributes"),
            prune_root.as_std_path().join(".gitattributes"),
        )
        .unwrap();
        std::fs::copy(
            pkg_dir.as_std_path().join("package.json"),
            prune_pkg.as_std_path().join("package.json"),
        )
        .unwrap();
        std::fs::copy(
            pkg_dir.as_std_path().join("readme.md"),
            prune_pkg.as_std_path().join("readme.md"),
        )
        .unwrap();

        let manual_hashes = get_package_file_hashes_without_git::<&str>(
            &prune_root,
            &pkg_path,
            &[],
            false,
            None,
            None,
        )
        .unwrap();

        let key = RelativeUnixPathBuf::new("readme.md").unwrap();
        let git_hash = git_hashes.get(&key).expect("readme.md in git hashes");
        let manual_hash = manual_hashes.get(&key).expect("readme.md in manual hashes");

        assert_eq!(
            git_hash, manual_hash,
            "package-scoped .gitattributes pattern must produce matching hashes. git={git_hash}, \
             manual={manual_hash}"
        );
    }

    /// Verify that hash_files (used for global dependencies) respects
    /// .gitattributes CRLF normalization.
    #[test]
    fn test_hash_files_respects_gitattributes() {
        let (_tmp, root) = tmp_dir();

        std::fs::write(root.as_std_path().join(".gitattributes"), "* text=auto\n").unwrap();

        let crlf_file = root.join_component("global-dep.env");
        std::fs::write(crlf_file.as_std_path(), b"KEY=value\r\nOTHER=val\r\n").unwrap();

        let lf_file = root.join_component("lockfile.lock");
        std::fs::write(lf_file.as_std_path(), b"dep=1.0\n").unwrap();

        // Initialize a git repo to get the expected normalized hashes
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root.as_std_path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "--local", "core.autocrlf", "false"])
            .current_dir(root.as_std_path())
            .output()
            .unwrap();

        // git hash-object --path applies .gitattributes filters (normalizes CRLF)
        for name in &["global-dep.env", "lockfile.lock"] {
            let git_output = Command::new("git")
                .args(["hash-object", "--path", name, "--stdin"])
                .stdin(std::process::Stdio::from(
                    std::fs::File::open(root.as_std_path().join(name)).unwrap(),
                ))
                .current_dir(root.as_std_path())
                .output()
                .unwrap();
            assert!(git_output.status.success());
            let expected = String::from_utf8(git_output.stdout).unwrap();
            let expected = expected.trim();

            let files = [AnchoredSystemPathBuf::from_raw(name).unwrap()];
            let hashes = crate::manual::hash_files(&root, files.iter(), false, None, None).unwrap();
            let key = RelativeUnixPathBuf::new(*name).unwrap();
            let actual = hashes.get(&key).expect("file should be in hashes");

            assert_eq!(
                &**actual, expected,
                "hash_files for {name} must match git hash-object --path (with filters)"
            );
        }
    }

    /// Verify CRLF normalization works correctly when core.autocrlf=true,
    /// which is the default Windows configuration. Git normalizes CRLF→LF
    /// in blobs when autocrlf is enabled, even without .gitattributes. Our
    /// normalization relies on .gitattributes, so this test validates the
    /// combined scenario (autocrlf=true + text=auto).
    #[test]
    fn test_crlf_hash_with_autocrlf_true() {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let pkg_dir = repo_root.join_component("my-pkg");
        pkg_dir.create_dir_all().unwrap();

        std::fs::write(
            repo_root.as_std_path().join(".gitattributes"),
            "* text=auto\n",
        )
        .unwrap();

        std::fs::write(
            pkg_dir.as_std_path().join("readme.md"),
            b"Hello\r\nWorld\r\n",
        )
        .unwrap();
        std::fs::write(
            pkg_dir.as_std_path().join("package.json"),
            "{\"name\": \"my-pkg\"}\n",
        )
        .unwrap();

        setup_repository(&repo_root);
        // Enable autocrlf — the real Windows scenario
        require_git_cmd(&repo_root, &["config", "--local", "core.autocrlf", "true"]);
        commit_all(&repo_root);

        let pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg").unwrap();

        // Hash via git path (with autocrlf=true)
        let git = SCM::new(&repo_root);
        let git_hashes = git
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[], false, None, None)
            .unwrap();

        // Hash via manual path
        let (_prune_tmp, prune_root) = tmp_dir();
        let prune_pkg = prune_root.join_component("my-pkg");
        prune_pkg.create_dir_all().unwrap();
        std::fs::copy(
            repo_root.as_std_path().join(".gitattributes"),
            prune_root.as_std_path().join(".gitattributes"),
        )
        .unwrap();
        std::fs::copy(
            pkg_dir.as_std_path().join("package.json"),
            prune_pkg.as_std_path().join("package.json"),
        )
        .unwrap();
        std::fs::copy(
            pkg_dir.as_std_path().join("readme.md"),
            prune_pkg.as_std_path().join("readme.md"),
        )
        .unwrap();

        let manual_hashes = get_package_file_hashes_without_git::<&str>(
            &prune_root,
            &pkg_path,
            &[],
            false,
            None,
            None,
        )
        .unwrap();

        let key = RelativeUnixPathBuf::new("readme.md").unwrap();
        let git_hash = git_hashes.get(&key).expect("readme.md in git");
        let manual_hash = manual_hashes.get(&key).expect("readme.md in manual");

        assert_eq!(
            git_hash, manual_hash,
            "hashes must match with core.autocrlf=true + text=auto. git={git_hash}, \
             manual={manual_hash}"
        );
    }
}
