#![cfg(feature = "git2")]
use std::str::FromStr;

use globwalk::{GlobSet, ValidatedGlob};
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, PathError};
use turborepo_telemetry::events::task::{FileHashMethod, PackageTaskEventBuilder};

#[cfg(feature = "git2")]
use crate::hash_object::hash_objects;
use crate::{Error, GitHashes, GitRepo, RepoGitIndex, SCM};

/// Bundle of pre-compiled glob sets for include and exclude patterns.
/// Passed through to avoid redundant glob compilation when multiple
/// packages share the same task input patterns.
pub struct PrecompiledGlobs<'a> {
    pub include: Option<&'a GlobSet>,
    pub exclude: Option<&'a GlobSet>,
}

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

    /// Like `get_package_file_hashes`, but accepts pre-compiled glob sets
    /// to avoid redundant glob compilation when multiple packages share the
    /// same input patterns.
    pub fn get_package_file_hashes_with_precompiled<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        include_default_files: bool,
        repo_index: Option<&RepoGitIndex>,
        precompiled: &PrecompiledGlobs<'_>,
    ) -> Result<GitHashes, Error> {
        match self {
            SCM::Manual => crate::manual::get_package_file_hashes_without_git(
                turbo_root,
                package_path,
                inputs,
                include_default_files,
            ),
            SCM::Git(git) => {
                let result = git.get_package_file_hashes_precompiled(
                    turbo_root,
                    package_path,
                    inputs,
                    include_default_files,
                    repo_index,
                    precompiled,
                );
                match result {
                    Ok(hashes) => Ok(hashes),
                    Err(err) => {
                        if err.is_resource_exhaustion() {
                            return Err(err);
                        }
                        debug!(
                            "git hashing failed for {:?}: {}. Falling back to manual",
                            package_path, err,
                        );
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

    fn get_package_file_hashes_precompiled<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        include_default_files: bool,
        repo_index: Option<&RepoGitIndex>,
        precompiled: &PrecompiledGlobs<'_>,
    ) -> Result<GitHashes, Error> {
        if inputs.is_empty() {
            return self.get_package_file_hashes_from_index(turbo_root, package_path, repo_index);
        }

        if !include_default_files {
            return match precompiled.include {
                Some(gs) => self.get_package_file_hashes_from_inputs_precompiled(
                    turbo_root,
                    package_path,
                    gs,
                ),
                None => {
                    self.get_package_file_hashes_from_inputs(turbo_root, package_path, inputs, true)
                }
            };
        }

        self.get_package_file_hashes_from_inputs_and_index_precompiled(
            turbo_root,
            package_path,
            inputs,
            repo_index,
            precompiled,
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

        let mut inputs = inputs
            .iter()
            .map(|s| s.as_ref().to_string())
            .collect::<Vec<String>>();

        if include_configs {
            // Add in package.json and turbo.json to input patterns. Both file paths are
            // relative to pkgPath
            //
            // - package.json is an input because if the `scripts` in the package.json
            //   change (i.e. the tasks that turbo executes), we want a cache miss, since
            //   any existing cache could be invalid.
            // - turbo.json because it's the definition of the tasks themselves. The root
            //   turbo.json is similarly included in the global hash. This file may not
            //   exist in the workspace, but that is ok, because it will get ignored
            //   downstream.
            inputs.push("package.json".to_string());
            inputs.push("turbo.json".to_string());
            inputs.push("turbo.jsonc".to_string());
        }

        // The input patterns are relative to the package.
        // However, we need to change the globbing to be relative to the repo root.
        // Prepend the package path to each of the input patterns.
        //
        // FIXME: we don't yet error on absolute unix paths being passed in as inputs,
        // and instead tack them on as if they were relative paths. This should be an
        // error further upstream, but since we haven't pulled the switch yet,
        // we need to mimic the Go behavior here and trim leading `/`
        // characters.
        let mut inclusions = vec![];
        let mut exclusions = vec![];
        for raw_glob in inputs {
            if let Some(exclusion) = raw_glob.strip_prefix('!') {
                let glob_str = [package_unix_path, exclusion.trim_start_matches('/')].join("/");
                exclusions.push(ValidatedGlob::from_str(&glob_str)?);
            } else {
                let glob_str = [package_unix_path, raw_glob.trim_start_matches('/')].join("/");
                inclusions.push(ValidatedGlob::from_str(&glob_str)?);
            }
        }
        let files = globwalk::globwalk(
            turbo_root,
            &inclusions,
            &exclusions,
            globwalk::WalkType::Files,
        )?;
        let to_hash = files
            .iter()
            .map(|entry| {
                let path = self.root.anchor(entry)?.to_unix();
                Ok(path)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut hashes = GitHashes::new();
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
        // that a user explicitly wants to track). We still need globwalk for these
        // but can skip re-hashing files already known from the index.
        if !includes.is_empty() {
            let include_hashes = self.get_package_file_hashes_from_inputs(
                turbo_root,
                package_path,
                &includes,
                true,
            )?;
            hashes.extend(include_hashes);
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

    /// Like `get_package_file_hashes_from_inputs` but uses a pre-compiled
    /// `GlobSet` instead of compiling patterns from scratch.
    fn get_package_file_hashes_from_inputs_precompiled(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        glob_set: &GlobSet,
    ) -> Result<GitHashes, Error> {
        let full_pkg_path = turbo_root.resolve(package_path);

        let files = glob_set.walk(&full_pkg_path, globwalk::WalkType::Files)?;

        let to_hash = files
            .iter()
            .map(|entry| {
                let path = self.root.anchor(entry)?.to_unix();
                Ok(path)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut hashes = GitHashes::new();
        hash_objects(&self.root, &full_pkg_path, to_hash, &mut hashes)?;
        Ok(hashes)
    }

    /// Like `get_package_file_hashes_from_inputs_and_index` but uses
    /// pre-compiled glob sets for both include walking and exclude
    /// matching.
    fn get_package_file_hashes_from_inputs_and_index_precompiled<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        repo_index: Option<&RepoGitIndex>,
        precompiled: &PrecompiledGlobs<'_>,
    ) -> Result<GitHashes, Error> {
        let mut hashes =
            self.get_package_file_hashes_from_index(turbo_root, package_path, repo_index)?;

        let mut has_includes = false;
        let mut has_excludes = false;
        for input in inputs {
            let input_str = input.as_ref();
            if input_str.starts_with('!') {
                has_excludes = true;
            } else {
                has_includes = true;
            }
        }

        if has_includes {
            match precompiled.include {
                Some(gs) => {
                    let include_hashes = self.get_package_file_hashes_from_inputs_precompiled(
                        turbo_root,
                        package_path,
                        gs,
                    )?;
                    hashes.extend(include_hashes);
                }
                None => {
                    let includes: Vec<&str> = inputs
                        .iter()
                        .map(|s| s.as_ref())
                        .filter(|s| !s.starts_with('!'))
                        .collect();
                    let include_hashes = self.get_package_file_hashes_from_inputs(
                        turbo_root,
                        package_path,
                        &includes,
                        true,
                    )?;
                    hashes.extend(include_hashes);
                }
            }
        }

        if has_excludes {
            match precompiled.exclude {
                Some(gs) => {
                    hashes.retain(|key, _| !gs.matches(key.as_str()));
                }
                None => {
                    let excludes: Vec<&str> = inputs
                        .iter()
                        .map(|s| s.as_ref())
                        .filter_map(|s| s.strip_prefix('!'))
                        .collect();

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

    /// Verify that the precompiled GlobSet path produces identical hashes
    /// to the original per-package compilation path. This is the key
    /// correctness guarantee for the glob compilation cache.
    #[test]
    fn test_precompiled_matches_original_with_includes() -> Result<(), Error> {
        let (_repo_root_tmp, repo_root) = tmp_dir();

        // Setup git repo with multiple packages sharing the same inputs
        setup_repository(&repo_root);

        let packages = &["pkg-a", "pkg-b"];
        for pkg in packages {
            let pkg_dir = repo_root.join_component(pkg);
            pkg_dir.create_dir_all()?;
            pkg_dir
                .join_component("package.json")
                .create_with_contents("{}")?;
            pkg_dir
                .join_component("turbo.json")
                .create_with_contents("{}")?;
            let src_dir = pkg_dir.join_component("src");
            src_dir.create_dir_all()?;
            src_dir
                .join_component("index.ts")
                .create_with_contents("export default 1;")?;
            src_dir
                .join_component("utils.ts")
                .create_with_contents("export const x = 2;")?;
        }
        commit_all(&repo_root);

        let git = SCM::new(&repo_root);
        let SCM::Git(ref git_repo) = git else {
            panic!("expected git");
        };

        let inputs = &["src/**"];

        // Pre-compile a GlobSet for these patterns (with config files)
        let mut include_strs: Vec<String> = inputs.iter().map(|s| s.to_string()).collect();
        include_strs.push("package.json".to_string());
        include_strs.push("turbo.json".to_string());
        include_strs.push("turbo.jsonc".to_string());

        let inc_validated: Vec<globwalk::ValidatedGlob> = include_strs
            .iter()
            .filter_map(|s| std::str::FromStr::from_str(s).ok())
            .collect();
        let glob_set =
            globwalk::GlobSet::compile(&inc_validated, &[]).expect("compile should succeed");

        for pkg in packages {
            let pkg_path = AnchoredSystemPathBuf::from_raw(pkg)?;

            // Original path
            let original = git_repo
                .get_package_file_hashes_from_inputs(&repo_root, &pkg_path, inputs, true)?;

            // Precompiled path
            let precompiled = git_repo.get_package_file_hashes_from_inputs_precompiled(
                &repo_root, &pkg_path, &glob_set,
            )?;

            assert_eq!(
                original, precompiled,
                "precompiled and original hashes differ for {pkg}"
            );
            assert!(!original.is_empty(), "expected non-empty hashes for {pkg}");
        }
        Ok(())
    }

    /// Verify precompiled path produces identical results when exclude patterns
    /// are used alongside the default file index.
    #[test]
    fn test_precompiled_matches_original_with_includes_and_excludes() -> Result<(), Error> {
        let (_repo_root_tmp, repo_root) = tmp_dir();
        setup_repository(&repo_root);

        let pkg_dir = repo_root.join_component("my-pkg");
        pkg_dir.create_dir_all()?;
        pkg_dir
            .join_component("package.json")
            .create_with_contents("{}")?;
        pkg_dir
            .join_component("turbo.json")
            .create_with_contents("{}")?;
        let src_dir = pkg_dir.join_component("src");
        src_dir.create_dir_all()?;
        src_dir
            .join_component("index.ts")
            .create_with_contents("main")?;
        let gen_dir = src_dir.join_component("generated");
        gen_dir.create_dir_all()?;
        gen_dir
            .join_component("types.ts")
            .create_with_contents("generated")?;
        commit_all(&repo_root);

        let git = SCM::new(&repo_root);
        let SCM::Git(ref git_repo) = git else {
            panic!("expected git");
        };

        let inputs: &[&str] = &["src/**", "!src/generated/**"];
        let pkg_path = AnchoredSystemPathBuf::from_raw("my-pkg")?;

        // Original path (inputs_and_index with excludes)
        let original = git_repo.get_package_file_hashes(
            &repo_root, &pkg_path, inputs, true, // include_default_files
            None,
        )?;

        // Precompiled path: build include and exclude glob sets
        let mut include_strs: Vec<String> = Vec::new();
        let mut exclude_strs: Vec<String> = Vec::new();
        for input in inputs {
            if let Some(excl) = input.strip_prefix('!') {
                exclude_strs.push(excl.to_string());
            } else {
                include_strs.push(input.to_string());
            }
        }
        include_strs.push("package.json".to_string());
        include_strs.push("turbo.json".to_string());
        include_strs.push("turbo.jsonc".to_string());

        let inc_v: Vec<globwalk::ValidatedGlob> = include_strs
            .iter()
            .filter_map(|s| std::str::FromStr::from_str(s).ok())
            .collect();
        let exc_v: Vec<globwalk::ValidatedGlob> = exclude_strs
            .iter()
            .filter_map(|s| std::str::FromStr::from_str(s).ok())
            .collect();
        let include_gs = globwalk::GlobSet::compile(&inc_v, &exc_v).expect("include compile");
        let ex_v: Vec<globwalk::ValidatedGlob> = exclude_strs
            .iter()
            .filter_map(|s| std::str::FromStr::from_str(s).ok())
            .collect();
        let exclude_gs = globwalk::GlobSet::compile(&ex_v, &[]).expect("exclude compile");

        let precompiled_globs = PrecompiledGlobs {
            include: Some(&include_gs),
            exclude: Some(&exclude_gs),
        };
        let precompiled = git_repo.get_package_file_hashes_precompiled(
            &repo_root,
            &pkg_path,
            inputs,
            true,
            None,
            &precompiled_globs,
        )?;

        assert_eq!(
            original, precompiled,
            "precompiled with excludes should match original"
        );

        // Verify the exclude actually worked: generated/types.ts should not be present
        assert!(
            !original.keys().any(|k| k.as_str().contains("generated")),
            "excluded files should not appear in hashes: {:?}",
            original.keys().collect::<Vec<_>>()
        );

        Ok(())
    }
}
