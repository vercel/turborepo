use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
};

use nom::Finish;
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};

use crate::{wait_for_success, Error, Git};

impl Git {
    #[tracing::instrument(skip(self, root_path))]
    pub(crate) fn append_git_status(
        &self,
        root_path: &AbsoluteSystemPath,
        pkg_prefix: &RelativeUnixPathBuf,
    ) -> Result<(Vec<RelativeUnixPathBuf>, Vec<RelativeUnixPathBuf>), Error> {
        let mut git = Command::new(self.bin.as_std_path())
            .args([
                "status",
                "--untracked-files",
                "--no-renames",
                "-z",
                "--",
                ".",
            ])
            .current_dir(root_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = git
            .stdout
            .as_mut()
            .ok_or_else(|| Error::git_error("failed to get stdout for git status"))?;
        let mut stderr = git
            .stderr
            .take()
            .ok_or_else(|| Error::git_error("failed to get stderr for git status"))?;
        let parse_result = read_status(stdout, root_path, pkg_prefix);
        wait_for_success(git, &mut stderr, "git status", root_path, parse_result)
    }
}

fn read_status<R: Read>(
    reader: R,
    root_path: &AbsoluteSystemPath,
    pkg_prefix: &RelativeUnixPathBuf,
) -> Result<(Vec<RelativeUnixPathBuf>, Vec<RelativeUnixPathBuf>), Error> {
    let mut to_hash = Vec::new();
    let mut to_remove = Vec::new();
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    while reader.read_until(b'\0', &mut buffer)? != 0 {
        let entry = parse_status(&buffer)?;
        let path = RelativeUnixPathBuf::new(String::from_utf8(entry.filename.to_owned())?)?;
        if entry.is_delete {
            let path = path.strip_prefix(pkg_prefix).map_err(|_| {
                Error::git_error(format!(
                    "'git status --untracked-files --no-renames -z -- .' run in {} found a \
                     deleted file {} that did not have the expected prefix: {}",
                    root_path, path, pkg_prefix
                ))
            })?;
            to_remove.push(path);
        } else {
            to_hash.push(path);
        }
        buffer.clear();
    }
    Ok((to_hash, to_remove))
}

struct StatusEntry<'a> {
    filename: &'a [u8],
    is_delete: bool,
}

fn parse_status(i: &[u8]) -> Result<StatusEntry<'_>, Error> {
    match nom::combinator::all_consuming(nom_parse_status)(i).finish() {
        Ok((_, tup)) => Ok(tup),
        Err(e) => Err(Error::git_error(format!(
            "failed to parse git-status: {}",
            String::from_utf8_lossy(e.input)
        ))),
    }
}

fn nom_parse_status(i: &[u8]) -> nom::IResult<&[u8], StatusEntry<'_>> {
    let (i, x) = nom::bytes::complete::take(1usize)(i)?;
    let (i, y) = nom::bytes::complete::take(1usize)(i)?;
    let (i, _) = nom::character::complete::space1(i)?;
    let (i, filename) = nom::bytes::complete::is_not(" \0")(i)?;
    // We explicitly support a missing terminator
    let (i, _) = nom::combinator::opt(nom::bytes::complete::tag(&[b'\0']))(i)?;
    Ok((
        i,
        StatusEntry {
            filename,
            is_delete: x[0] == b'D' || y[0] == b'D',
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf, RelativeUnixPathBufTestExt};

    use super::read_status;
    use crate::package_deps::GitHashes;

    #[test]
    fn test_status() {
        let root_path = AbsoluteSystemPathBuf::cwd().unwrap();
        let tests: &[(&str, &str, (&str, bool))] = &[
            ("AD my-pkg/package.json\0", "my-pkg", ("package.json", true)),
            (
                // no trailing NUL
                "AD some-pkg/package.json",
                "some-pkg",
                ("package.json", true),
            ),
            ("M  package.json\0", "", ("package.json", false)),
            ("A  some-pkg/some-file\0", "some-pkg", ("some-file", false)),
        ];
        for (input, prefix, (expected_filename, expect_delete)) in tests {
            let prefix = RelativeUnixPathBuf::new(*prefix).unwrap();
            let mut hashes = to_hash_map(&[(expected_filename, "some-hash")]);
            let to_hash = read_status(input.as_bytes(), &root_path, &prefix, &mut hashes).unwrap();
            if *expect_delete {
                assert_eq!(hashes.len(), 0, "input: {}", input);
            } else {
                assert_eq!(to_hash.len(), 1, "input: {}", input);
                let expected = prefix.join(&RelativeUnixPathBuf::new(*expected_filename).unwrap());
                assert_eq!(to_hash[0], expected);
            }
        }
    }

    fn to_hash_map(pairs: &[(&str, &str)]) -> GitHashes {
        HashMap::from_iter(
            pairs
                .iter()
                .map(|(path, hash)| (RelativeUnixPathBuf::new(*path).unwrap(), hash.to_string())),
        )
    }
}
