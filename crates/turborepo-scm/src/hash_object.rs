use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{package_deps::GitHashes, Error};

pub(crate) fn hash_objects(
    git_root: &AbsoluteSystemPath,
    pkg_path: &AbsoluteSystemPath,
    to_hash: Vec<RelativeUnixPathBuf>,
    hashes: &mut GitHashes,
) -> Result<(), Error> {
    for filename in to_hash {
        let full_file_path = git_root.join_unix_path(filename)?;
        let hash = git2::Oid::hash_file(git2::ObjectType::Blob, &full_file_path)?;
        let package_relative_path =
            AnchoredSystemPathBuf::relative_path_between(pkg_path, &full_file_path).to_unix()?;
        hashes.insert(package_relative_path, hash.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf, RelativeUnixPathBufTestExt};

    use super::hash_objects;
    use crate::package_deps::{find_git_root, GitHashes};

    #[test]
    fn test_read_object_hashes() {
        // Note that cwd can be different based on where the test suite is running from
        // or if the test is launched in debug mode from VSCode
        let cwd = std::env::current_dir().unwrap();
        let cwd = AbsoluteSystemPathBuf::new(cwd).unwrap();
        let git_root = find_git_root(&cwd).unwrap();
        let fixture_path = git_root.join_components(&[
            "crates",
            "turborepo-scm",
            "fixtures",
            "01-git-hash-object",
        ]);

        let fixture_child_path = fixture_path.join_component("child");
        let git_root = find_git_root(&fixture_path).unwrap();

        // paths for files here are relative to the package path.
        let tests: Vec<(Vec<(&str, &str)>, &AbsoluteSystemPathBuf)> = vec![
            (vec![], &fixture_path),
            (
                vec![
                    ("../root.json", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
                    ("child.json", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
                    (
                        "grandchild/grandchild.json",
                        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
                    ),
                ],
                &fixture_child_path,
            ),
        ];

        for (to_hash, pkg_path) in tests {
            let file_hashes: Vec<(RelativeUnixPathBuf, String)> = to_hash
                .into_iter()
                .map(|(raw, hash)| (RelativeUnixPathBuf::new(raw).unwrap(), String::from(hash)))
                .collect();

            let git_to_pkg_path = git_root.anchor(pkg_path).unwrap();
            let pkg_prefix = git_to_pkg_path.to_unix().unwrap();

            let expected_hashes = GitHashes::from_iter(file_hashes.into_iter());
            let mut hashes = GitHashes::new();
            let to_hash = expected_hashes.keys().map(|k| pkg_prefix.join(k)).collect();
            hash_objects(&git_root, pkg_path, to_hash, &mut hashes).unwrap();
            assert_eq!(hashes, expected_hashes);
        }

        // paths for files here are relative to the package path.
        let error_tests: Vec<(Vec<&str>, &AbsoluteSystemPathBuf)> = vec![
            // skipping test for outside of git repo, we now error earlier in the process
            (vec!["nonexistent.json"], &fixture_path),
        ];

        for (to_hash, pkg_path) in error_tests {
            let git_to_pkg_path = git_root.anchor(pkg_path).unwrap();
            let pkg_prefix = git_to_pkg_path.to_unix().unwrap();

            let to_hash = to_hash
                .into_iter()
                .map(|k| pkg_prefix.join(&RelativeUnixPathBuf::new(k).unwrap()))
                .collect();

            let mut hashes = GitHashes::new();
            let result = hash_objects(&git_root, &pkg_path, to_hash, &mut hashes);
            assert_eq!(result.is_err(), true);
        }
    }
}
