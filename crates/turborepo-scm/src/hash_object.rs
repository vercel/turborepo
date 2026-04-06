use rayon::prelude::*;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPath, RelativeUnixPathBuf};

use crate::{Error, GitHashes, OidHash};

const MAX_RETRIES: u32 = 10;
const BASE_DELAY_MS: u64 = 10;
const MAX_DELAY_MS: u64 = 1000;

fn with_emfile_retry<T>(f: impl Fn() -> Result<T, std::io::Error>) -> Result<T, std::io::Error> {
    for attempt in 0..MAX_RETRIES {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if is_too_many_open_files(&e) => {
                let delay = std::cmp::min(BASE_DELAY_MS * 2u64.pow(attempt), MAX_DELAY_MS);
                debug!(
                    attempt = attempt + 1,
                    delay_ms = delay,
                    "too many open files, retrying"
                );
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
            Err(e) => return Err(e),
        }
    }
    f()
}

fn is_too_many_open_files(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(24)) // EMFILE
        || e.to_string().contains("Too many open files")
}

/// Hash a batch of files as git blob objects, applying CRLF→LF
/// normalization when `.gitattributes` requires it.
///
/// `cached_attrs` reuses a pre-loaded [`crate::crlf::GitAttrs`] when
/// available (e.g. from the `GitRepo`'s `OnceLock`). When `None`, attrs
/// are loaded per-batch from `git_root`.
#[tracing::instrument(skip(git_root, hashes, to_hash, cached_attrs))]
pub(crate) fn hash_objects(
    git_root: &AbsoluteSystemPath,
    pkg_path: &AbsoluteSystemPath,
    to_hash: Vec<RelativeUnixPathBuf>,
    hashes: &mut GitHashes,
    cached_attrs: Option<&crate::crlf::GitAttrs>,
) -> Result<(), Error> {
    let pkg_prefix = git_root.anchor(pkg_path).ok().map(|a| a.to_unix());

    let mut owned_attrs = None;
    let attrs = crate::crlf::resolve_or_load(cached_attrs, git_root, &mut owned_attrs);

    hashes.reserve(to_hash.len());
    let results: Vec<Result<Option<(RelativeUnixPathBuf, OidHash)>, Error>> = to_hash
        .into_par_iter()
        // `map_init` creates one Outcome per rayon thread, reused across all
        // files in that thread's partition. This avoids a per-file allocation
        // inside `resolve_text_attr`.
        .map_init(
            || attrs.map(|a| a.new_outcome()),
            |outcome, filename| {
                let full_file_path = git_root.join_unix_path(&filename);

                let text_attr = match (attrs, outcome.as_mut()) {
                    (Some(a), Some(o)) => a.resolve_text_attr_with(filename.as_str(), o),
                    _ => crate::crlf::TextAttr::Unspecified,
                };

                let hash_result = with_emfile_retry(|| {
                    crate::crlf::hash_file_maybe_normalized(&full_file_path, text_attr)
                });

                match hash_result {
                    Ok(hash) => {
                        let package_relative_path = pkg_prefix
                            .as_ref()
                            .and_then(|prefix| {
                                RelativeUnixPath::strip_prefix(&filename, prefix)
                                    .ok()
                                    .map(|stripped| stripped.to_owned())
                            })
                            .unwrap_or_else(|| {
                                AnchoredSystemPathBuf::relative_path_between(
                                    pkg_path,
                                    &full_file_path,
                                )
                                .to_unix()
                            });
                        let mut hex_buf = [0u8; 40];
                        hex::encode_to_slice(hash.as_bytes(), &mut hex_buf).unwrap();
                        Ok(Some((
                            package_relative_path,
                            OidHash::from_hex_buf(hex_buf),
                        )))
                    }
                    Err(e) => {
                        // Gracefully skip non-regular files (symlinks, sockets,
                        // FIFOs, device nodes) that can't be read as normal files.
                        if full_file_path
                            .symlink_metadata()
                            .map(|md| !md.is_file())
                            .unwrap_or(false)
                        {
                            Ok(None)
                        } else {
                            Err(Error::git_error(format!("{}: {}", full_file_path, e)))
                        }
                    }
                }
            },
        )
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
    use crate::{GitHashes, OidHash, find_git_root};

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
            let file_hashes: Vec<(RelativeUnixPathBuf, OidHash)> = to_hash
                .into_iter()
                .map(|(raw, hash)| {
                    (
                        RelativeUnixPathBuf::new(raw).unwrap(),
                        OidHash::from_hex_str(hash),
                    )
                })
                .collect();

            let git_to_pkg_path = git_root.anchor(pkg_path).unwrap();
            let pkg_prefix = git_to_pkg_path.to_unix();

            let expected_hashes = GitHashes::from_iter(file_hashes);
            let mut hashes = GitHashes::new();
            let to_hash = expected_hashes.keys().map(|k| pkg_prefix.join(k)).collect();
            hash_objects(&git_root, pkg_path, to_hash, &mut hashes, None).unwrap();
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
            let result = hash_objects(&git_root, pkg_path, to_hash, &mut hashes, None);
            assert!(result.is_err());
        }
    }

    /// Verify that our blob hashing produces OIDs identical to `git
    /// hash-object`. This is critical because changing the hash algorithm
    /// would silently invalidate every turbo cache entry.
    #[test]
    fn test_blob_hash_matches_git_hash_object() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

        // Initialize a git repo so hash_objects can run
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        // 128KB: spans multiple 64KB read buffers, exercising the streaming loop
        const MULTI_BUF: usize = 128 * 1024;
        let multi_buf_content = vec![b'A'; MULTI_BUF];
        // Exactly 64KB: boundary where one read fills the buffer and the next returns 0
        const EXACT_BUF: usize = 64 * 1024;
        let exact_buf_content = vec![b'B'; EXACT_BUF];

        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty.txt", b"".to_vec()),
            ("hello.txt", b"hello world\n".to_vec()),
            ("binary.bin", vec![0u8, 1, 2, 255, 254, 253]),
            ("large.txt", vec![b'x'; 10_000]),
            ("multi_buf.bin", multi_buf_content),
            ("exact_buf.bin", exact_buf_content),
            // CRLF edge cases: without --filters, git hash-object hashes raw
            // bytes. These must remain stable after CRLF normalization is added
            // to ensure we only normalize when .gitattributes says to.
            ("lone-cr.txt", b"hello\rworld\n".to_vec()),
            ("mixed-eol.txt", b"line1\nline2\r\nline3\n".to_vec()),
            ("trailing-cr.bin", b"data\r".to_vec()),
            ("crlf-in-binary.bin", vec![0x00, b'\r', b'\n', 0xFF, 0xFE]),
            ("pure-crlf.txt", b"a\r\nb\r\nc\r\n".to_vec()),
        ];

        for (name, content) in &cases {
            std::fs::write(tmp.path().join(name), content).unwrap();
        }

        // Get expected hashes from git itself
        let mut expected = GitHashes::new();
        for (name, _) in &cases {
            let output = std::process::Command::new("git")
                .args(["hash-object", name])
                .current_dir(tmp.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "git hash-object failed for {name}");
            let hash = String::from_utf8(output.stdout).unwrap();
            let hash = hash.trim();
            expected.insert(
                RelativeUnixPathBuf::new(*name).unwrap(),
                OidHash::from_hex_str(hash),
            );
        }

        // Hash with our implementation
        let to_hash: Vec<_> = cases
            .iter()
            .map(|(name, _)| RelativeUnixPathBuf::new(*name).unwrap())
            .collect();
        let mut actual = GitHashes::new();
        hash_objects(&tmp_path, &tmp_path, to_hash, &mut actual, None).unwrap();

        assert_eq!(actual, expected, "blob hashes must match git hash-object");
    }
}
