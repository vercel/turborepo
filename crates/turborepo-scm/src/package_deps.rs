use std::{collections::HashMap, process::Command};

use bstr::io::BufReadExt;
use itertools::{Either, Itertools};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf,
};

use crate::{hash_object::hash_objects, ls_tree::git_ls_tree, status::append_git_status, Error};

pub type GitHashes = HashMap<RelativeUnixPathBuf, String>;

pub fn get_package_file_hashes_from_git_index(
    turbo_root: &AbsoluteSystemPath,
    package_path: &AnchoredSystemPathBuf,
) -> Result<GitHashes, Error> {
    // TODO: memoize git root -> turbo root calculation once we aren't crossing ffi
    let git_root = find_git_root(turbo_root)?;
    let full_pkg_path = turbo_root.resolve(package_path);
    let git_to_pkg_path = git_root.anchor(&full_pkg_path)?;
    let pkg_prefix = git_to_pkg_path.to_unix()?;
    let mut hashes = git_ls_tree(&full_pkg_path)?;
    // Note: to_hash is *git repo relative*
    let to_hash = append_git_status(&full_pkg_path, &pkg_prefix, &mut hashes)?;
    hash_objects(&git_root, &full_pkg_path, to_hash, &mut hashes)?;
    Ok(hashes)
}

pub fn get_package_file_hashes_from_inputs<S: AsRef<str>>(
    turbo_root: &AbsoluteSystemPath,
    package_path: &AnchoredSystemPathBuf,
    inputs: &[S],
) -> Result<GitHashes, Error> {
    // TODO: memoize git root -> turbo root calculation once we aren't crossing ffi
    let git_root = find_git_root(turbo_root)?;
    let full_pkg_path = turbo_root.resolve(package_path);
    let package_unix_path_buf = package_path.to_unix()?;
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
    let (inclusions, exclusions): (Vec<String>, Vec<String>) =
        inputs.into_iter().partition_map(|raw_glob| {
            if let Some(exclusion) = raw_glob.strip_prefix('!') {
                Either::Right([package_unix_path, exclusion].join("/"))
            } else {
                Either::Left([package_unix_path, raw_glob.as_ref()].join("/"))
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
            let path = git_root.anchor(entry)?.to_unix()?;
            Ok(path)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let mut hashes = GitHashes::new();
    hash_objects(&git_root, &full_pkg_path, to_hash, &mut hashes)?;
    Ok(hashes)
}

pub(crate) fn find_git_root(
    turbo_root: &AbsoluteSystemPath,
) -> Result<AbsoluteSystemPathBuf, Error> {
    let rev_parse = Command::new("git")
        .args(["rev-parse", "--show-cdup"])
        .current_dir(turbo_root)
        .output()?;
    if !rev_parse.status.success() {
        let stderr = String::from_utf8_lossy(&rev_parse.stderr);
        return Err(Error::git_error(format!(
            "git rev-parse --show-cdup error: {}",
            stderr
        )));
    }
    let cursor = std::io::Cursor::new(rev_parse.stdout);
    let mut lines = cursor.byte_lines();
    if let Some(line) = lines.next() {
        let line = String::from_utf8(line?)?;
        let tail = RelativeUnixPathBuf::new(line)?;
        turbo_root.join_unix_path(tail).map_err(|e| e.into())
    } else {
        let stderr = String::from_utf8_lossy(&rev_parse.stderr);
        Err(Error::git_error(format!(
            "git rev-parse --show-cdup error: no values on stdout. stderr: {}",
            stderr
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, process::Command};

    use super::*;
    use crate::manual::get_package_file_hashes_from_processing_gitignore;

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
            get_package_file_hashes_from_processing_gitignore(&git_root, &pkg_path, &["l*"])
                .unwrap();
        assert!(manual_hashes.is_empty());
    }

    #[test]
    fn test_symlinked_git_root() {
        let (_, tmp_root) = tmp_dir();
        let git_root = tmp_root.join_component("actual_repo");
        git_root.create_dir_all().unwrap();
        setup_repository(&git_root);
        git_root.join_component("inside").create_dir_all().unwrap();
        let link = tmp_root.join_component("link");
        link.symlink_to_dir("actual_repo").unwrap();
        let turbo_root = link.join_component("inside");
        let result = find_git_root(&turbo_root).unwrap();
        assert_eq!(result, link);
    }

    #[test]
    fn test_no_git_root() {
        let (_, tmp_root) = tmp_dir();
        tmp_root.create_dir_all().unwrap();
        let result = find_git_root(&tmp_root);
        assert_matches!(result, Err(Error::Git(_, _)));
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
        let hashes = get_package_file_hashes_from_git_index(&repo_root, &package_path)?;
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
            let expected: GitHashes = HashMap::from_iter(expected_files.into_iter().map(|key| {
                let key = RelativeUnixPathBuf::new(*key).unwrap();
                let value = all_expected.get(&key).unwrap().clone();
                (key, value)
            }));
            let hashes =
                get_package_file_hashes_from_inputs(&repo_root, &package_path, &inputs).unwrap();
            assert_eq!(hashes, expected);
        }
        Ok(())
    }

    fn to_hash_map(pairs: &[(&str, &str)]) -> GitHashes {
        HashMap::from_iter(
            pairs
                .into_iter()
                .map(|(path, hash)| (RelativeUnixPathBuf::new(*path).unwrap(), hash.to_string())),
        )
    }
}
