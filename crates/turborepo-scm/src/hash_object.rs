use std::collections::HashMap;

use rayon::prelude::*;
use tracing::{debug, info};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPath,
    RelativeUnixPathBuf,
};

use crate::{Error, GitHashes, OidHash};

const MAX_RETRIES: u32 = 10;
const BASE_DELAY_MS: u64 = 10;
const MAX_DELAY_MS: u64 = 1000;

#[derive(Clone, Copy)]
enum MissingFiles {
    Error,
    Ignore,
}

pub(crate) fn with_emfile_retry<T>(
    f: impl Fn() -> Result<T, std::io::Error>,
) -> Result<T, std::io::Error> {
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
    slowest_files: Option<&std::sync::Arc<crate::SlowestFiles>>,
) -> Result<(), Error> {
    hash_objects_inner(
        git_root,
        git_root,
        pkg_path,
        to_hash,
        hashes,
        MissingFiles::Error,
        cached_attrs,
        slowest_files,
    )
}

pub(crate) fn hash_discovered_objects(
    root: &AbsoluteSystemPath,
    git_root: &AbsoluteSystemPath,
    pkg_path: &AbsoluteSystemPath,
    to_hash: Vec<RelativeUnixPathBuf>,
    hashes: &mut GitHashes,
    cached_attrs: Option<&crate::crlf::GitAttrs>,
    slowest_files: Option<&std::sync::Arc<crate::SlowestFiles>>,
) -> Result<(), Error> {
    hash_objects_inner(
        root,
        git_root,
        pkg_path,
        to_hash,
        hashes,
        MissingFiles::Ignore,
        cached_attrs,
        slowest_files,
    )
}

#[allow(clippy::too_many_arguments)]
fn hash_objects_inner(
    root: &AbsoluteSystemPath,
    git_root: &AbsoluteSystemPath,
    pkg_path: &AbsoluteSystemPath,
    to_hash: Vec<RelativeUnixPathBuf>,
    hashes: &mut GitHashes,
    missing_files: MissingFiles,
    cached_attrs: Option<&crate::crlf::GitAttrs>,
    slowest_files: Option<&std::sync::Arc<crate::SlowestFiles>>,
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

                let _guard = slowest_files.map(|sf| sf.start(filename.clone()));
                let hash_result = with_emfile_retry(|| {
                    crate::crlf::hash_file_as_git_blob(root, &full_file_path, text_attr)
                });
                drop(_guard);

                match hash_result {
                    Ok(Some(hash)) => {
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
                        Ok(Some((package_relative_path, hash)))
                    }
                    Ok(None) => {
                        info!(path = %full_file_path, "skipping non-regular hash candidate");
                        Ok(None)
                    }
                    Err(error)
                        if matches!(missing_files, MissingFiles::Ignore)
                            && error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        info!(path = %full_file_path, "discovered hash candidate disappeared");
                        Ok(None)
                    }
                    Err(error) => Err(Error::hash_file(full_file_path, error)),
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

/// Whether an input's logical path crosses a symlink below the trusted root.
pub(crate) fn has_symlink(root: &AbsoluteSystemPath, path: &AbsoluteSystemPath) -> bool {
    let mut current = Some(path);
    while let Some(path) = current.filter(|path| *path != root) {
        if path
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return true;
        }
        current = path.parent();
    }
    false
}

/// Reuses ancestor checks only within a single tracked-file selection pass.
/// The leaf is always checked afresh; directories shared by sibling files are
/// checked once. Do not retain this across hashing operations, since links can
/// change between runs.
pub(crate) struct SymlinkCache<'a> {
    root: &'a AbsoluteSystemPath,
    ancestors: HashMap<AbsoluteSystemPathBuf, bool>,
}

impl<'a> SymlinkCache<'a> {
    pub(crate) fn new(root: &'a AbsoluteSystemPath) -> Self {
        Self {
            root,
            ancestors: HashMap::new(),
        }
    }

    pub(crate) fn has_symlink(&mut self, path: &AbsoluteSystemPath) -> bool {
        if path == self.root {
            return false;
        }
        if path
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return true;
        }

        let mut current = path.parent();
        let mut visited = Vec::new();
        let mut linked = false;
        while let Some(parent) = current.filter(|parent| *parent != self.root) {
            if let Some(&cached) = self.ancestors.get(parent) {
                linked = cached;
                break;
            }
            visited.push(parent);
            if parent
                .symlink_metadata()
                .is_ok_and(|meta| meta.file_type().is_symlink())
            {
                linked = true;
                break;
            }
            current = parent.parent();
        }
        for parent in visited {
            self.ancestors.insert(parent.to_owned(), linked);
        }
        linked
    }
}

#[cfg(test)]
mod test {
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPathBuf, RelativeUnixPathBufTestExt};

    use super::{SymlinkCache, has_symlink, hash_discovered_objects, hash_objects};
    use crate::{GitHashes, OidHash, find_git_root};

    #[test]
    fn test_cached_symlink_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let nested = root.join_components(&["src", "nested"]);
        nested.create_dir_all().unwrap();
        nested
            .join_component("file")
            .create_with_contents("data")
            .unwrap();
        let paths = [
            "src/nested/file",
            "src/nested/missing",
            "missing/child",
            "src",
            "",
        ];
        let mut cache = SymlinkCache::new(&root);
        for _ in 0..2 {
            for relative in paths {
                let path = root.join_components(&relative.split('/').collect::<Vec<_>>());
                assert_eq!(cache.has_symlink(&path), has_symlink(&root, &path));
            }
        }
        assert!(!cache.ancestors.contains_key(&root));
        assert!(!cache.ancestors.contains_key(&nested.join_component("file")));
    }

    #[cfg(unix)]
    #[test]
    fn test_cached_symlink_ancestors_and_fresh_leaves() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let nested = root.join_components(&["src", "nested"]);
        nested.create_dir_all().unwrap();
        let file = nested.join_component("file");
        file.create_with_contents("data").unwrap();
        root.join_component("linked").symlink_to_dir("src").unwrap();
        root.join_component("dangling")
            .symlink_to_file("absent")
            .unwrap();
        root.join_component("cycle")
            .symlink_to_dir("cycle")
            .unwrap();
        let paths = [
            "linked/nested/file",
            "linked/nested/missing",
            "linked/other",
            "dangling",
            "dangling/child",
            "cycle/child",
            "src/nested/file",
        ];
        let mut cache = SymlinkCache::new(&root);
        for _ in 0..2 {
            for relative in paths {
                let path = root.join_components(&relative.split('/').collect::<Vec<_>>());
                assert_eq!(
                    cache.has_symlink(&path),
                    has_symlink(&root, &path),
                    "{relative}"
                );
            }
        }
        assert!(!cache.has_symlink(&file));
        file.remove().unwrap();
        file.symlink_to_file("missing").unwrap();
        assert!(cache.has_symlink(&file), "leaf checks must not be cached");

        // A new pass must observe changes to directories too.
        root.join_component("linked").remove().unwrap();
        root.join_component("linked").create_dir_all().unwrap();
        let mut cache = SymlinkCache::new(&root);
        assert!(!cache.has_symlink(&root.join_components(&["linked", "missing"])));

        // A symlink at the trusted root is deliberately not inspected.
        let trusted = tmp.path().join("trusted");
        std::os::unix::fs::symlink(&nested, &trusted).unwrap();
        let trusted = AbsoluteSystemPathBuf::try_from(trusted).unwrap();
        let mut cache = SymlinkCache::new(&trusted);
        assert!(!cache.has_symlink(&trusted));
        assert!(!cache.has_symlink(&trusted.join_component("missing")));
    }

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
            hash_objects(&git_root, pkg_path, to_hash, &mut hashes, None, None).unwrap();
            assert_eq!(hashes, expected_hashes);
        }
    }

    #[test]
    fn test_vanished_candidate_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let git_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let candidate = git_root.join_component("transient.lock");
        candidate.create_with_contents("lock").unwrap();

        // Model a glob walk finding a transient file that disappears before
        // the parallel blob-hashing phase starts.
        let to_hash = vec![RelativeUnixPathBuf::new("transient.lock").unwrap()];
        candidate.remove().unwrap();

        let mut hashes = GitHashes::new();
        hash_discovered_objects(
            &git_root,
            &git_root,
            &git_root,
            to_hash,
            &mut hashes,
            None,
            None,
        )
        .unwrap();
        assert!(hashes.is_empty());
    }

    #[test]
    fn test_missing_explicit_file_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let git_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let missing = RelativeUnixPathBuf::new("missing.json").unwrap();

        let mut hashes = GitHashes::new();
        let error =
            hash_objects(&git_root, &git_root, vec![missing], &mut hashes, None, None).unwrap_err();

        assert!(error.to_string().contains("missing.json"));
    }

    #[cfg(unix)]
    #[test]
    fn test_non_regular_symlink_is_skipped() {
        use std::os::unix::net::UnixListener;

        let tmp = tempfile::tempdir().unwrap();
        let git_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let socket = git_root.join_component("server.sock");
        let _listener = UnixListener::bind(socket.as_std_path()).unwrap();
        let link = git_root.join_component("socket-link");
        link.symlink_to_file(socket.to_string()).unwrap();

        let mut hashes = GitHashes::new();
        hash_objects(
            &git_root,
            &git_root,
            vec![RelativeUnixPathBuf::new("socket-link").unwrap()],
            &mut hashes,
            None,
            None,
        )
        .unwrap();

        assert!(hashes.is_empty());
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
        hash_objects(&tmp_path, &tmp_path, to_hash, &mut actual, None, None).unwrap();

        assert_eq!(actual, expected, "blob hashes must match git hash-object");
    }
}
