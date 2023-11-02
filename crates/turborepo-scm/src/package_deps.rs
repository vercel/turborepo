use std::collections::HashMap;

use itertools::{Either, Itertools};
use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, PathError, RelativeUnixPathBuf};

use crate::{hash_object::hash_objects, Error, Git, SCM};

pub type GitHashes = HashMap<RelativeUnixPathBuf, String>;

/// Precalculates workspace data where it makes sense to reduce overhead from
/// lazily calculating it on demand.
pub enum CachedPackageFileHasher<'a> {
    Manual,
    Git(GitCachedPackageFileHasher<'a>),
}

impl<'a> CachedPackageFileHasher<'a> {
    /// When creating a new CachedPackageFileHasher with the git hashes, we
    /// precalculate and store a datastructure that maps package paths to
    /// hashes of the files in that package, and a list of files that
    /// changed. This saves having to call out to git for each package.
    pub fn new(
        scm: &'a SCM,
        repo_root: &AbsoluteSystemPath,
        package_roots: impl Iterator<Item = &'a AnchoredSystemPath>,
    ) -> Self {
        match scm {
            SCM::Git(git) => {
                let mut map: HashMap<&AnchoredSystemPath, (GitHashes, Vec<_>)> = Default::default();

                let j1 = {
                    let git = git.clone();
                    let repo_root = repo_root.to_owned();
                    std::thread::spawn(move || git.git_ls_tree(&repo_root).unwrap())
                };

                let j2 = {
                    let git = git.clone();
                    let repo_root = repo_root.to_owned();
                    std::thread::spawn(move || {
                        git.append_git_status(&repo_root, &Default::default())
                            .unwrap()
                    })
                };

                let (path_hashes, to_hash) = {
                    let mut path_hashes = j1.join().unwrap();
                    let (to_hash, to_remove) = j2.join().unwrap();

                    for path in to_remove {
                        path_hashes.remove(&path);
                    }
                    (path_hashes, to_hash)
                };

                let mut hash_trie = qp_trie::Trie::new();
                for (path, hash) in path_hashes {
                    hash_trie.insert_str(path.as_str(), hash);
                }

                let mut file_trie = qp_trie::Trie::new();
                for path in &to_hash {
                    file_trie.insert_str(path.as_str(), ());
                }

                // a subpackage will always have a longer length than its parent,
                // so we sort by length to ensure that we always and drain subpackages
                // before their parents
                for package in package_roots {
                    let package_str = package.as_str();

                    let (hashes, files) = map.entry(package).or_default();

                    hashes.extend(hash_trie.subtrie_str(package.as_str()).iter().filter_map(
                        |(path, hash)| {
                            path.as_str()
                                .strip_prefix(package_str)
                                .expect("path is a subpath of package")
                                .strip_prefix('/') // don't include the package itself
                                .map(|p| {
                                    (
                                        RelativeUnixPathBuf::new(p).expect("relative"),
                                        hash.to_owned(),
                                    )
                                })
                        },
                    ));

                    files.extend(file_trie.subtrie_str(package.as_str()).iter().filter_map(
                        |(path, _)| {
                            path.as_str()
                                .strip_prefix(package_str)
                                .expect("path is a subpath of package")
                                .strip_prefix('/') // don't include the package itself
                                .map(|p| RelativeUnixPathBuf::new(p).expect("relative"))
                        },
                    ));
                }

                Self::Git(GitCachedPackageFileHasher { git, map })
            }
            SCM::Manual => Self::Manual,
        }
    }

    pub fn get_package_file_hashes<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
    ) -> Result<GitHashes, Error> {
        match self {
            CachedPackageFileHasher::Manual => {
                crate::manual::get_package_file_hashes_from_processing_gitignore(
                    turbo_root,
                    package_path,
                    inputs,
                )
            }
            CachedPackageFileHasher::Git(git) => {
                git.get_package_file_hashes(turbo_root, package_path, inputs)
            }
        }
    }
}

pub struct GitCachedPackageFileHasher<'a> {
    git: &'a Git,

    /// A map of package paths to hashes of the files in that package, and a
    /// list of files that changed
    map: HashMap<&'a AnchoredSystemPath, (GitHashes, Vec<RelativeUnixPathBuf>)>,
}

impl<'a> GitCachedPackageFileHasher<'a> {
    fn get_package_file_hashes<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &'a AnchoredSystemPath,
        inputs: &[S],
    ) -> Result<GitHashes, Error> {
        if inputs.is_empty() {
            // cached branch
            self.get_hashes_for_path(turbo_root, package_path)
        } else {
            self.git
                .get_package_file_hashes_from_inputs(turbo_root, package_path, inputs)
        }
    }

    fn get_hashes_for_path(
        &self,
        turbo_root: &AbsoluteSystemPath,

        package_path: &AnchoredSystemPath,
    ) -> Result<GitHashes, Error> {
        // if in the cache, use it, otherwise fall back
        let Some((hashes, files)) = self.map.get(package_path) else {
            warn!("git cache miss for package path {:?}", package_path);
            return self
                .git
                .get_package_file_hashes_from_index(turbo_root, package_path);
        };

        let mut hashes = hashes.clone();
        let to_hash = files.iter().map(|f| f.as_ref()).collect::<Vec<_>>();
        hash_objects(
            &self.git.root,
            &self.git.root.resolve(package_path),
            &to_hash,
            &mut hashes,
        )?;

        debug_assert!({
            let expected = self
                .git
                .get_package_file_hashes_from_index(turbo_root, package_path);
            if expected.is_err() {
                panic!("error when fetching hashes");
                // false
            } else {
                assert_eq!(
                    expected.unwrap(),
                    hashes,
                    "hashes match for {:?}",
                    package_path
                );
                true
            }
        });

        Ok(hashes)
    }
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

    #[tracing::instrument(skip(self, turbo_root, package_path, inputs))]
    pub fn get_package_file_hashes<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
    ) -> Result<GitHashes, Error> {
        match self {
            SCM::Manual => crate::manual::get_package_file_hashes_from_processing_gitignore(
                turbo_root,
                package_path,
                inputs,
            ),
            SCM::Git(git) => git
                .get_package_file_hashes(turbo_root, package_path, inputs)
                .or_else(|e| {
                    debug!(
                        "failed to use git to hash files: {}. Falling back to manual",
                        e
                    );
                    crate::manual::get_package_file_hashes_from_processing_gitignore(
                        turbo_root,
                        package_path,
                        inputs,
                    )
                }),
        }
    }

    /// Prepare a cached package hasher.
    ///
    /// note: package_roots must be sorted alphabetically
    #[tracing::instrument(skip(package_roots))]
    pub fn prepare_cached_package_file_hasher<'a>(
        &'a self,
        repo_root: &AbsoluteSystemPath,
        package_roots: impl Iterator<Item = &'a AnchoredSystemPath>,
    ) -> CachedPackageFileHasher<'a> {
        CachedPackageFileHasher::new(self, repo_root, package_roots)
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
    ) -> Result<GitHashes, Error> {
        if inputs.is_empty() {
            self.get_package_file_hashes_from_index(turbo_root, package_path)
        } else {
            self.get_package_file_hashes_from_inputs(turbo_root, package_path, inputs)
        }
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
        let (to_hash, to_remove) = self.append_git_status(&full_pkg_path, &pkg_prefix)?;
        for path in to_remove {
            hashes.remove(&path);
        }

        hash_objects(&self.root, &full_pkg_path, &to_hash, &mut hashes)?;
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
        hash_objects(&self.root, process_relative_to, &to_hash, &mut hashes)?;
        Ok(hashes)
    }

    #[tracing::instrument(skip(self, turbo_root, inputs))]
    fn get_package_file_hashes_from_inputs<S: AsRef<str>>(
        &self,
        turbo_root: &AbsoluteSystemPath,
        package_path: &AnchoredSystemPath,
        inputs: &[S],
    ) -> Result<GitHashes, Error> {
        let full_pkg_path = turbo_root.resolve(package_path);
        let package_unix_path_buf = package_path.to_unix();
        let package_unix_path = package_unix_path_buf.as_str();

        let mut inputs = inputs
            .iter()
            .map(|s| s.as_ref().to_string())
            .collect::<Vec<String>>();
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

        // The input patterns are relative to the package.
        // However, we need to change the globbing to be relative to the repo root.
        // Prepend the package path to each of the input patterns.
        //
        // FIXME: we don't yet error on absolute unix paths being passed in as inputs,
        // and instead tack them on as if they were relative paths. This should be an
        // error further upstream, but since we haven't pulled the switch yet,
        // we need to mimic the Go behavior here and trim leading `/`
        // characters.
        let (inclusions, exclusions): (Vec<String>, Vec<String>) =
            inputs.into_iter().partition_map(|raw_glob| {
                if let Some(exclusion) = raw_glob.strip_prefix('!') {
                    Either::Right([package_unix_path, exclusion.trim_start_matches('/')].join("/"))
                } else {
                    Either::Left([package_unix_path, raw_glob.trim_start_matches('/')].join("/"))
                }
            });
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
        hash_objects(&self.root, &full_pkg_path, &to_hash, &mut hashes)?;
        Ok(hashes)
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, collections::HashMap, process::Command};

    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf};

    use super::*;
    use crate::{manual::get_package_file_hashes_from_processing_gitignore, SCM};

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
        hash_objects(&git_root, &git_root, &to_hash, &mut hashes).unwrap();
        assert!(hashes.is_empty());

        let pkg_path = git_root.anchor(&git_root).unwrap();
        let manual_hashes =
            get_package_file_hashes_from_processing_gitignore(&git_root, &pkg_path, &["l*"])
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
            .get_package_file_hashes::<&str>(&repo_root, &pkg_path, &[])
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
        //   new-root-file <- new file not added to git
        //   my-pkg/
        //     committed-file
        //     deleted-file
        //     uncommitted-file <- new file not added to git
        //     dir/
        //       nested-file
        let (_repo_root_tmp, repo_root) = tmp_dir();
        let my_pkg_dir = repo_root.join_component("my-pkg");
        my_pkg_dir.create_dir_all()?;

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

        // create a package.json
        let pkg_json_path = my_pkg_dir.join_component("package.json");
        pkg_json_path.create_with_contents("{}")?;

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

        let package_path = AnchoredSystemPathBuf::from_raw("my-pkg")?;

        let all_expected = to_hash_map(&[
            ("committed-file", "3a29e62ea9ba15c4a4009d1f605d391cdd262033"),
            (
                "uncommitted-file",
                "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
            ),
            ("package.json", "9e26dfeeb6e641a33dae4961196235bdb965b21b"),
            (
                "dir/nested-file",
                "bfe53d766e64d78f80050b73cd1c88095bc70abb",
            ),
        ]);
        let hashes = git.get_package_file_hashes::<&str>(&repo_root, &package_path, &[])?;
        assert_eq!(hashes, all_expected);

        // add the new root file as an option
        let mut all_expected = all_expected.clone();
        all_expected.insert(
            RelativeUnixPathBuf::new("../new-root-file").unwrap(),
            "8906ddcdd634706188bd8ef1c98ac07b9be3425e".to_string(),
        );

        let input_tests: &[(&[&str], &[&str])] = &[
            (&["uncommitted-file"], &["package.json", "uncommitted-file"]),
            (
                &["**/*-file"],
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "dir/nested-file",
                ],
            ),
            (
                &["../**/*-file"],
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "dir/nested-file",
                    "../new-root-file",
                ],
            ),
            (
                &["**/{uncommitted,committed}-file"],
                &["committed-file", "uncommitted-file", "package.json"],
            ),
            (
                &["../**/{new-root,uncommitted,committed}-file"],
                &[
                    "committed-file",
                    "uncommitted-file",
                    "package.json",
                    "../new-root-file",
                ],
            ),
        ];
        for (inputs, expected_files) in input_tests {
            let expected: GitHashes = HashMap::from_iter(expected_files.iter().map(|key| {
                let key = RelativeUnixPathBuf::new(*key).unwrap();
                let value = all_expected.get(&key).unwrap().clone();
                (key, value)
            }));
            let hashes = git
                .get_package_file_hashes(&repo_root, &package_path, inputs)
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
