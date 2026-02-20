use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
};

use nom::Finish;
use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf};

use crate::{wait_for_success, Error, GitHashes, GitRepo};

/// Sorted list of (path, hash) pairs from `git ls-tree`. Uses a `Vec` instead
/// of `BTreeMap` because git output is already sorted by pathname, giving us
/// free insertion order with better cache locality for the `partition_point`
/// range lookups performed in `RepoGitIndex::get_package_hashes`.
pub(crate) type SortedGitHashes = Vec<(RelativeUnixPathBuf, String)>;

impl GitRepo {
    #[tracing::instrument(skip(self))]
    pub fn git_ls_tree(&self, root_path: &AbsoluteSystemPathBuf) -> Result<GitHashes, Error> {
        let mut hashes = GitHashes::new();
        let mut git = Command::new(self.bin.as_std_path())
            .args(["ls-tree", "-r", "-z", "HEAD"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .current_dir(root_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = git
            .stdout
            .as_mut()
            .ok_or_else(|| Error::git_error("failed to get stdout for git ls-tree"))?;
        let mut stderr = git
            .stderr
            .take()
            .ok_or_else(|| Error::git_error("failed to get stderr for git ls-tree"))?;
        let parse_result = read_ls_tree(stdout, &mut hashes);
        wait_for_success(git, &mut stderr, "git ls-tree", root_path, parse_result)?;
        Ok(hashes)
    }

    /// Run `git ls-tree` once at the git repo root, returning all committed
    /// file hashes in a sorted Vec for efficient prefix-range lookups.
    ///
    /// Uses libgit2 to walk the HEAD tree in-process, avoiding the overhead
    /// of spawning a git subprocess.
    #[cfg(feature = "git2")]
    #[tracing::instrument(skip(self))]
    pub fn git_ls_tree_repo_root_sorted(&self) -> Result<SortedGitHashes, Error> {
        let repo = git2::Repository::open(self.root.as_std_path())
            .map_err(|e| Error::git2_error_context(e, "opening repo for ls-tree".into()))?;
        let head = repo
            .head()
            .map_err(|e| Error::git2_error_context(e, "resolving HEAD".into()))?;
        let tree = head
            .peel_to_tree()
            .map_err(|e| Error::git2_error_context(e, "peeling HEAD to tree".into()))?;

        let mut hashes = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            // Only collect blob entries (files), skip trees (directories)
            if entry.kind() == Some(git2::ObjectType::Blob) {
                let name = match entry.name() {
                    Some(n) => n,
                    None => return git2::TreeWalkResult::Ok,
                };
                let path_str = if dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{dir}{name}")
                };
                if let Ok(path) = RelativeUnixPathBuf::new(path_str) {
                    hashes.push((path, entry.id().to_string()));
                }
            }
            git2::TreeWalkResult::Ok
        })
        .map_err(|e| Error::git2_error_context(e, "walking tree".into()))?;

        // git2 tree walk is in pre-order which is lexicographic within each
        // directory level, but the flattened paths may not be globally sorted
        // (e.g. "a/b" vs "a.txt"). Sort to maintain the binary-search invariant.
        hashes.sort_by(|(a, _), (b, _)| a.cmp(b));

        Ok(hashes)
    }

    /// Run `git ls-tree` once at the git repo root, returning all committed
    /// file hashes keyed by git-root-relative paths.
    #[tracing::instrument(skip(self))]
    pub fn git_ls_tree_repo_root(&self) -> Result<GitHashes, Error> {
        self.git_ls_tree(&self.root)
    }
}

fn read_ls_tree<R: Read>(reader: R, hashes: &mut GitHashes) -> Result<(), Error> {
    let mut reader = BufReader::with_capacity(64 * 1024, reader);
    let mut buffer = Vec::new();
    while reader.read_until(b'\0', &mut buffer)? != 0 {
        let entry = parse_ls_tree(&buffer)?;
        let hash = std::str::from_utf8(entry.hash)
            .map_err(|e| Error::git_error(format!("invalid utf8 in ls-tree hash: {e}")))?;
        let filename = std::str::from_utf8(entry.filename)
            .map_err(|e| Error::git_error(format!("invalid utf8 in ls-tree filename: {e}")))?;
        let path = RelativeUnixPathBuf::new(filename)?;
        hashes.insert(path, hash.to_owned());
        buffer.clear();
    }
    Ok(())
}

#[cfg(test)]
fn read_ls_tree_sorted<R: Read>(reader: R, hashes: &mut SortedGitHashes) -> Result<(), Error> {
    let mut reader = BufReader::with_capacity(64 * 1024, reader);
    let mut buffer = Vec::new();
    while reader.read_until(b'\0', &mut buffer)? != 0 {
        let entry = parse_ls_tree(&buffer)?;
        let hash = std::str::from_utf8(entry.hash)
            .map_err(|e| Error::git_error(format!("invalid utf8 in ls-tree hash: {e}")))?;
        let filename = std::str::from_utf8(entry.filename)
            .map_err(|e| Error::git_error(format!("invalid utf8 in ls-tree filename: {e}")))?;
        let path = RelativeUnixPathBuf::new(filename)?;
        hashes.push((path, hash.to_owned()));
        buffer.clear();
    }
    debug_assert!(
        hashes.windows(2).all(|w| w[0].0 < w[1].0),
        "git ls-tree output should be sorted by pathname"
    );
    Ok(())
}

struct LsTreeEntry<'a> {
    filename: &'a [u8],
    hash: &'a [u8],
}

fn parse_ls_tree(i: &[u8]) -> Result<LsTreeEntry<'_>, Error> {
    let mut parser = nom::combinator::all_consuming(nom_parse_ls_tree);
    match parser(i).finish() {
        Ok((_, entry)) => Ok(entry),
        Err(e) => Err(Error::git_error(format!(
            "failed to parse git-ls-tree: {}",
            String::from_utf8_lossy(e.input)
        ))),
    }
}

fn nom_parse_ls_tree(i: &[u8]) -> nom::IResult<&[u8], LsTreeEntry<'_>> {
    let (i, _) = nom::bytes::complete::is_not(" ")(i)?;
    let (i, _) = nom::character::complete::space1(i)?;
    let (i, _) = nom::bytes::complete::is_not(" ")(i)?;
    let (i, _) = nom::character::complete::space1(i)?;
    let (i, hash) = nom::bytes::complete::take(40usize)(i)?;
    let (i, _) = nom::bytes::complete::take(1usize)(i)?;
    let (i, filename) = nom::bytes::complete::is_not("\0")(i)?;
    // We explicitly support a missing terminator
    let (i, _) = nom::combinator::opt(nom::bytes::complete::tag(b"\0"))(i)?;
    Ok((i, LsTreeEntry { filename, hash }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use turbopath::RelativeUnixPathBuf;

    use crate::{ls_tree::read_ls_tree, GitHashes};

    fn to_hash_map(pairs: &[(&str, &str)]) -> GitHashes {
        HashMap::from_iter(
            pairs
                .iter()
                .map(|(path, hash)| (RelativeUnixPathBuf::new(*path).unwrap(), hash.to_string())),
        )
    }

    fn to_sorted_hashes(pairs: &[(&str, &str)]) -> super::SortedGitHashes {
        pairs
            .iter()
            .map(|(path, hash)| (RelativeUnixPathBuf::new(*path).unwrap(), hash.to_string()))
            .collect()
    }

    // Verifies that read_ls_tree_sorted produces correct sorted Vec entries
    // from git ls-tree output.
    #[test]
    fn test_ls_tree_sorted() {
        let input = "100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\tpackage.json\x00100644 \
                     blob 5b999efa470b056e329b4c23a73904e0794bdc2f\tsrc/index.ts\x00100644 blob \
                     f44f57fff95196c5f7139dfa0b96875f1e9650a9\tsrc/utils.ts\0";

        let expected = to_sorted_hashes(&[
            ("package.json", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
            ("src/index.ts", "5b999efa470b056e329b4c23a73904e0794bdc2f"),
            ("src/utils.ts", "f44f57fff95196c5f7139dfa0b96875f1e9650a9"),
        ]);

        let mut hashes = super::SortedGitHashes::new();
        super::read_ls_tree_sorted(input.as_bytes(), &mut hashes).unwrap();
        assert_eq!(hashes, expected);

        // Verify entries are sorted (invariant needed for binary search)
        assert!(
            hashes.windows(2).all(|w| w[0].0 < w[1].0),
            "sorted Vec should maintain sorted order"
        );
    }

    // Verifies read_ls_tree_sorted handles all the edge cases that read_ls_tree
    // handles. Both parsers share the same `parse_ls_tree` function.
    #[test]
    fn test_ls_tree_sorted_edge_cases() {
        // Single entry without trailing NUL
        let input = "100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\tpackage.json";
        let mut hashes = super::SortedGitHashes::new();
        super::read_ls_tree_sorted(input.as_bytes(), &mut hashes).unwrap();
        assert_eq!(hashes.len(), 1);

        // Empty input
        let mut hashes = super::SortedGitHashes::new();
        super::read_ls_tree_sorted("".as_bytes(), &mut hashes).unwrap();
        assert_eq!(hashes.len(), 0);
    }

    #[test]
    fn test_ls_tree() {
        let tests: &[(&str, &[(&str, &str)])] = &[
            (
                "100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\tpackage.json\0",
                &[("package.json", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")],
            ),
            (
                // missing nul byte
                "100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\tpackage.json",
                &[("package.json", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")],
            ),
            (
                // We aren't attempting to use octal escapes here, it just looks like it
                #[allow(clippy::octal_escapes)]
                "100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\t\t\000100644 blob \
                 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\t\"\000100644 blob \
                 5b999efa470b056e329b4c23a73904e0794bdc2f\t\n\000100644 blob \
                 f44f57fff95196c5f7139dfa0b96875f1e9650a9\t.gitignore\000100644 blob \
                 33dbaf21275ca2a5f460249d941cbc27d5da3121\tREADME.md\000040000 tree \
                 7360f2d292aec95907cebdcbb412a6bf2bd10f8a\tapps\000100644 blob \
                 9ec2879b24ce2c817296eebe2cb3846f8e4751ea\tpackage.json\000040000 tree \
                 5759aadaea2cde55468a61e7104eb0a9d86c1d30\tpackages\000100644 blob \
                 33d0621ee2f4da4a2f6f6bdd51a42618d181e337\tturbo.json\000100644 blob \
                 579f273c9536d324c20b2e8f0d7fe4784ed0d9df\tfile with spaces\0",
                &[
                    ("\t", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
                    ("\"", "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
                    ("\n", "5b999efa470b056e329b4c23a73904e0794bdc2f"),
                    (".gitignore", "f44f57fff95196c5f7139dfa0b96875f1e9650a9"),
                    ("README.md", "33dbaf21275ca2a5f460249d941cbc27d5da3121"),
                    ("apps", "7360f2d292aec95907cebdcbb412a6bf2bd10f8a"),
                    ("package.json", "9ec2879b24ce2c817296eebe2cb3846f8e4751ea"),
                    ("packages", "5759aadaea2cde55468a61e7104eb0a9d86c1d30"),
                    ("turbo.json", "33d0621ee2f4da4a2f6f6bdd51a42618d181e337"),
                    (
                        "file with spaces",
                        "579f273c9536d324c20b2e8f0d7fe4784ed0d9df",
                    ),
                ],
            ),
        ];
        for (input, expected) in tests {
            let input_bytes = input.as_bytes();
            let mut hashes = GitHashes::new();
            let expected = to_hash_map(expected);
            read_ls_tree(input_bytes, &mut hashes).unwrap();
            assert_eq!(hashes, expected);
        }
    }
}
