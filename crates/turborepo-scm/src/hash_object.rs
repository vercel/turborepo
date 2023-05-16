use std::{
    io::{BufWriter, Read, Write},
    panic,
    process::{Command, Stdio},
    thread,
};

use nom::{Finish, IResult};
use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};

use crate::{package_deps::GitHashes, Error};

pub(crate) fn hash_objects(
    pkg_path: &AbsoluteSystemPathBuf,
    to_hash: Vec<RelativeUnixPathBuf>,
    pkg_prefix: &RelativeUnixPathBuf,
    hashes: &mut GitHashes,
) -> Result<(), Error> {
    if to_hash.is_empty() {
        return Ok(());
    }
    let mut git = Command::new("git")
        .args(["hash-object", "--stdin-paths"])
        .current_dir(pkg_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()?;
    {
        let stdout = git
            .stdout
            .as_mut()
            .ok_or_else(|| Error::git_error("failed to get stdout for git hash-object"))?;
        // We take, rather than borrow, stdin so that we can drop it and force the
        // underlying file descriptor to close, signalling the end of input.
        let stdin: std::process::ChildStdin = git
            .stdin
            .take()
            .ok_or_else(|| Error::git_error("failed to get stdin for git hash-object"))?;
        let mut stderr = git
            .stderr
            .take()
            .ok_or_else(|| Error::git_error("failed to get stderr for git hash-object"))?;
        let result = read_object_hashes(stdout, stdin, &to_hash, pkg_prefix, hashes);
        if let Err(err) = result {
            let mut buf = String::new();
            let bytes_read = stderr.read_to_string(&mut buf)?;
            if bytes_read > 0 {
                // something failed with git, report that error
                return Err(Error::git_error(buf));
            }
            return Err(err);
        }
    }
    git.wait()?;
    Ok(())
}

const HASH_LEN: usize = 40;

fn read_object_hashes<R: Read, W: Write + Send>(
    mut reader: R,
    writer: W,
    to_hash: &Vec<RelativeUnixPathBuf>,
    pkg_prefix: &RelativeUnixPathBuf,
    hashes: &mut GitHashes,
) -> Result<(), Error> {
    thread::scope(move |scope| -> Result<(), Error> {
        let write_thread = scope.spawn(move || -> Result<(), Error> {
            let mut writer = BufWriter::new(writer);
            for path in to_hash {
                path.write_escaped_bytes(&mut writer)?;
                writer.write_all(&[b'\n'])?;
                writer.flush()?;
            }
            // writer is dropped here, closing stdin
            Ok(())
        });
        // Buffer size is HASH_LEN + 1 to account for the trailing \n
        let mut buffer: [u8; HASH_LEN + 1] = [0; HASH_LEN + 1];
        for (i, filename) in to_hash.iter().enumerate() {
            if i == to_hash.len() {
                break;
            }
            reader.read_exact(&mut buffer)?;
            {
                let hash = parse_hash_object(&buffer)?;
                let hash = String::from_utf8(hash.to_vec())?;
                let path = filename.strip_prefix(pkg_prefix)?;
                hashes.insert(path, hash);
            }
        }
        match write_thread.join() {
            // the error case is if the thread panic'd. In that case, we propagate
            // the panic, since we aren't going to handle it.
            Err(e) => panic::resume_unwind(e),
            Ok(result) => result,
        }
    })?;
    Ok(())
}

fn parse_hash_object(i: &[u8]) -> Result<&[u8], Error> {
    match nom_parse_hash_object(i).finish() {
        Ok((_, hash)) => Ok(hash),
        Err(e) => Err(Error::git_error(format!(
            "failed to parse git-hash-object {}",
            String::from_utf8_lossy(e.input)
        ))),
    }
}

fn nom_parse_hash_object(i: &[u8]) -> IResult<&[u8], &[u8]> {
    let (i, hash) = nom::bytes::complete::take(HASH_LEN)(i)?;
    let (i, _) = nom::bytes::complete::tag(&[b'\n'])(i)?;
    Ok((i, hash))
}

#[cfg(test)]
mod test {
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};

    use super::hash_objects;
    use crate::package_deps::{find_git_root, GitHashes};

    #[test]
    fn test_read_object_hashes() {
        // Note that cwd can be different based on where the test suite is running from
        // or if the test is launched in debug mode from VSCode
        let cwd = std::env::current_dir().unwrap();
        let cwd = AbsoluteSystemPathBuf::new(cwd).unwrap();
        let git_root = find_git_root(&cwd).unwrap();
        let fixture_path = git_root
            .join_unix_path_literal("crates/turborepo-scm/fixtures/01-git-hash-object")
            .unwrap();

        let fixture_child_path = fixture_path.join_literal("child");
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
            hash_objects(&pkg_path, to_hash, &pkg_prefix, &mut hashes).unwrap();
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
            let result = hash_objects(&pkg_path, to_hash, &pkg_prefix, &mut hashes);
            assert_eq!(result.is_err(), true);
        }
    }
}
