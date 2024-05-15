use std::{collections::HashMap, str::FromStr};

use globwalk::ValidatedGlob;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, PathError, RelativeUnixPathBuf};
use turborepo_telemetry::events::task::{FileHashMethod, PackageTaskEventBuilder};

use crate::{hash_object::hash_objects, Error, Git, SCM};

pub type GitHashes = HashMap<RelativeUnixPathBuf, String>;

pub const INPUT_INCLUDE_DEFAULT_FILES: &str = "$TURBO_DEFAULT$";

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

    #[tracing::instrument(skip(self, turbo_root, package_path, inputs))]
    pub fn get_package_file_hashes<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        telemetry: Option<PackageTaskEventBuilder>,
    ) -> Result<GitHashes, Error> {
        // If the inputs contain "$TURBO_DEFAULT$", we need to include the "default"
        // file hashes as well. NOTE: we intentionally don't remove
        // "$TURBO_DEFAULT$" from the inputs if it exists in the off chance that
        // the user has a file named "$TURBO_DEFAULT$" in their package (pls
        // no).
        let include_default_files = inputs
            .iter()
            .any(|input| input.as_ref() == INPUT_INCLUDE_DEFAULT_FILES);

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
                );
                match result {
                    Ok(hashes) => {
                        if let Some(telemetry) = telemetry {
                            telemetry.track_file_hash_method(FileHashMethod::Git);
                        }
                        Ok(hashes)
                    }
                    Err(err) => {
                        debug!(
                            "failed to use git to hash files: {}. Falling back to manual",
                            err
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

impl Git {
    fn get_package_file_hashes<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
        include_default_files: bool,
    ) -> Result<GitHashes, Error> {
        // no inputs, and no $TURBO_DEFAULT$
        if inputs.is_empty() {
            return self.get_package_file_hashes_from_index(turbo_root, package_path);
        }

        // we have inputs, but no $TURBO_DEFAULT$
        if !include_default_files {
            return self.get_package_file_hashes_from_inputs(
                turbo_root,
                package_path,
                inputs,
                true,
            );
        }

        // we have inputs, and $TURBO_DEFAULT$
        self.get_package_file_hashes_from_inputs_and_index(turbo_root, package_path, inputs)
    }

    #[tracing::instrument(skip(self, turbo_root))]
    fn get_package_file_hashes_from_index(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
    ) -> Result<GitHashes, Error> {
        let full_pkg_path = turbo_root.resolve(package_path);
        let git_to_pkg_path = self.root.anchor(&full_pkg_path)?;
        let pkg_prefix = git_to_pkg_path.to_unix();
        let mut hashes = self.git_ls_tree(&full_pkg_path)?;
        // Note: to_hash is *git repo relative*
        let to_hash = self.append_git_status(&full_pkg_path, &pkg_prefix, &mut hashes)?;
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

    #[tracing::instrument(skip(self, turbo_root, inputs))]
    fn get_package_file_hashes_from_inputs_and_index<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
    ) -> Result<GitHashes, Error> {
        // collect the default files and the inputs
        let default_file_hashes =
            self.get_package_file_hashes_from_index(turbo_root, package_path)?;

        // we need to get hashes for excludes separately so we can remove them from the
        // defaults later on
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
        // we have to always run the includes search because we add default files to the
        // includes
        let manual_includes_hashes =
            self.get_package_file_hashes_from_inputs(turbo_root, package_path, &includes, true)?;

        // only run the excludes search if there are excludes
        let manual_excludes_hashes = if !excludes.is_empty() {
            self.get_package_file_hashes_from_inputs(turbo_root, package_path, &excludes, false)?
        } else {
            GitHashes::new()
        };

        // merge the two includes
        let mut hashes = default_file_hashes;
        hashes.extend(manual_includes_hashes);

        // remove the excludes
        hashes.retain(|key, _| !manual_excludes_hashes.contains_key(key));

        return Ok(hashes);
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, process::Command};

    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

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
                Some(PackageTaskEventBuilder::new("my-pkg", "test")),
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
            panic!("expected git, found {:?}", git);
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
        let hashes = git.get_package_file_hashes::<&str>(&repo_root, &package_path, &[], false)?;
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

        let input_tests: &[(&[&str], &[&str])] = &[
            (
                &["uncommitted-file"],
                &["package.json", "turbo.json", "uncommitted-file"],
            ),
            (
                &["**/*-file"],
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
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "turbo.json",
                ],
            ),
            (
                &["../**/{new-root,uncommitted,committed}-file"],
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
        for (inputs, expected_files) in input_tests {
            let expected: GitHashes = HashMap::from_iter(expected_files.iter().map(|key| {
                let key = RelativeUnixPathBuf::new(*key).unwrap();
                let value = all_expected.get(&key).unwrap().clone();
                (key, value)
            }));
            let include_default_files = inputs
                .iter()
                .any(|input| input == &INPUT_INCLUDE_DEFAULT_FILES);

            let hashes = git
                .get_package_file_hashes(&repo_root, &package_path, inputs, include_default_files)
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
