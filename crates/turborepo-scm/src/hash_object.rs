#![cfg(feature = "git2")]
use std::fmt::Write;

use rayon::prelude::*;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPath, RelativeUnixPathBuf};

use crate::{Error, GitHashes};

const MAX_RETRIES: u32 = 10;
const BASE_DELAY_MS: u64 = 10;
const MAX_DELAY_MS: u64 = 1000;

fn hash_file_with_retry(path: &AbsoluteSystemPath) -> Result<git2::Oid, git2::Error> {
    for attempt in 0..MAX_RETRIES {
        match git2::Oid::hash_file(git2::ObjectType::Blob, path) {
            Ok(oid) => return Ok(oid),
            Err(e) if is_too_many_open_files(&e) => {
                let delay = std::cmp::min(BASE_DELAY_MS * 2u64.pow(attempt), MAX_DELAY_MS);
                debug!(
                    attempt = attempt + 1,
                    delay_ms = delay,
                    "too many open files, retrying hash_file"
                );
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
            Err(e) => return Err(e),
        }
    }
    git2::Oid::hash_file(git2::ObjectType::Blob, path)
}

fn is_too_many_open_files(e: &git2::Error) -> bool {
    if e.class() != git2::ErrorClass::Os {
        return false;
    }
    let msg = e.message();
    msg.contains("Too many open files") || msg.contains("EMFILE")
}

#[tracing::instrument(skip(git_root, hashes, to_hash))]
pub(crate) fn hash_objects(
    git_root: &AbsoluteSystemPath,
    pkg_path: &AbsoluteSystemPath,
    to_hash: Vec<RelativeUnixPathBuf>,
    hashes: &mut GitHashes,
) -> Result<(), Error> {
    let pkg_prefix = git_root.anchor(pkg_path).ok().map(|a| a.to_unix());

    hashes.reserve(to_hash.len());
    let results: Vec<Result<Option<(RelativeUnixPathBuf, String)>, Error>> = to_hash
        .into_par_iter()
        .map(|filename| {
            let full_file_path = git_root.join_unix_path(&filename);
            match hash_file_with_retry(&full_file_path) {
                Ok(hash) => {
                    let package_relative_path = pkg_prefix
                        .as_ref()
                        .and_then(|prefix| {
                            RelativeUnixPath::strip_prefix(&filename, prefix)
                                .ok()
                                .map(|stripped| stripped.to_owned())
                        })
                        .unwrap_or_else(|| {
                            AnchoredSystemPathBuf::relative_path_between(pkg_path, &full_file_path)
                                .to_unix()
                        });
                    // Format the OID hex directly into a pre-sized String to
                    // avoid the intermediate allocations of Display + to_string().
                    let mut hex = String::with_capacity(40);
                    write!(hex, "{hash}").expect("writing to String cannot fail");
                    Ok(Some((package_relative_path, hex)))
                }
                Err(e) => {
                    if e.class() == git2::ErrorClass::Os
                        && full_file_path
                            .symlink_metadata()
                            .map(|md| md.is_symlink())
                            .unwrap_or(false)
                    {
                        Ok(None)
                    } else {
                        Err(Error::git2_error_context(e, full_file_path.to_string()))
                    }
                }
            }
        })
        .collect();

    for result in results {
        if let Some((path, hash)) = result? {
            hashes.insert(path, hash);
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf, RelativeUnixPathBufTestExt};

    use super::hash_objects;
    use crate::{GitHashes, find_git_root};

    #[test]
    fn test_read_object_hashes() {
        // Note that cwd can be different based on where the test suite is running from
        // or if the test is launched in debug mode from VSCode
        let cwd = std::env::current_dir().unwrap();
        let cwd = AbsoluteSystemPathBuf::try_from(cwd).unwrap();
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
            let pkg_prefix = git_to_pkg_path.to_unix();

            let expected_hashes = GitHashes::from_iter(file_hashes);
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
            let pkg_prefix = git_to_pkg_path.to_unix();

            let to_hash = to_hash
                .into_iter()
                .map(|k| pkg_prefix.join(&RelativeUnixPathBuf::new(k).unwrap()))
                .collect();

            let mut hashes = GitHashes::new();
            let result = hash_objects(&git_root, pkg_path, to_hash, &mut hashes);
            assert!(result.is_err());
        }
    }
}
