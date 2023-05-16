use std::{collections::HashMap, process::Command};

use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{hash_object::hash_objects, ls_tree::git_ls_tree, status::append_git_status, Error};

pub type GitHashes = HashMap<RelativeUnixPathBuf, String>;

pub fn get_package_file_hashes_from_git_index(
    turbo_root: &AbsoluteSystemPathBuf,
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
    hash_objects(&full_pkg_path, to_hash, &pkg_prefix, &mut hashes)?;
    Ok(hashes)
}

pub(crate) fn find_git_root(
    turbo_root: &AbsoluteSystemPathBuf,
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
    let root = String::from_utf8(rev_parse.stdout)?;
    Ok(turbo_root.join_literal(root.trim_end()).clean())
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, process::Command};

    use super::*;

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::new(tmp_dir.path().to_path_buf())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
    }

    fn require_git_cmd(repo_root: &AbsoluteSystemPathBuf, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(repo_root);
        assert_eq!(cmd.output().unwrap().status.success(), true);
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
    fn test_symlinked_git_root() {
        let (_, tmp_root) = tmp_dir();
        let git_root = tmp_root.join_literal("actual_repo");
        git_root.create_dir_all().unwrap();
        setup_repository(&git_root);
        git_root.join_literal("inside").create_dir_all().unwrap();
        let link = tmp_root.join_literal("link");
        link.symlink_to_dir("actual_repo").unwrap();
        let turbo_root = link.join_literal("inside");
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
        let my_pkg_dir = repo_root.join_literal("my-pkg");
        my_pkg_dir.create_dir_all()?;

        // create file 1
        let committed_file_path = my_pkg_dir.join_literal("committed-file");
        committed_file_path.create_with_contents("committed bytes")?;

        // create file 2
        let deleted_file_path = my_pkg_dir.join_literal("deleted-file");
        deleted_file_path.create_with_contents("delete-me")?;

        // create file 3
        let nested_file_path = my_pkg_dir.join_literal("dir/nested-file");
        nested_file_path.ensure_dir()?;
        nested_file_path.create_with_contents("nested")?;

        // create a package.json
        let pkg_json_path = my_pkg_dir.join_literal("package.json");
        pkg_json_path.create_with_contents("{}")?;

        setup_repository(&repo_root);
        commit_all(&repo_root);

        // remove a file
        deleted_file_path.remove()?;

        // create another untracked file in git
        let uncommitted_file_path = my_pkg_dir.join_literal("uncommitted-file");
        uncommitted_file_path.create_with_contents("uncommitted bytes")?;

        // create an untracked file in git up a level
        let root_file_path = repo_root.join_literal("new-root-file");
        root_file_path.create_with_contents("new-root bytes")?;

        let package_path = AnchoredSystemPathBuf::from_raw("my-pkg")?;

        let expected = to_hash_map(&[
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
        assert_eq!(hashes, expected);
        Ok(())
    }

    fn to_hash_map(pairs: &[(&str, &str)]) -> GitHashes {
        HashMap::from_iter(pairs.into_iter().map(|(path, hash)| {
            (
                RelativeUnixPathBuf::new(path.as_bytes()).unwrap(),
                hash.to_string(),
            )
        }))
    }
}
