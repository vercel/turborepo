use std::io::{ErrorKind, Read};

use globwalk::fix_glob_pattern;
use hex::ToHex;
use ignore::WalkBuilder;
use sha1::{Digest, Sha1};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, IntoUnix};
use wax::{any, Glob, Program};

use crate::{package_deps::GitHashes, Error};

fn git_like_hash_file(path: &AbsoluteSystemPath) -> Result<String, Error> {
    let mut hasher = Sha1::new();
    let mut f = path.open()?;
    let mut buffer = Vec::new();
    // Note that read_to_end reads the target if f is a symlink. Currently, this can
    // happen when we are hashing a specific set of files, which in turn only
    // happens for handling dotEnv files. It is likely that in the future we
    // will want to ensure that the target is better accounted for in the set of
    // inputs to the task. Manual hashing, as well as global deps and other
    // places that support globs all ignore symlinks.
    let size = f.read_to_end(&mut buffer)?;
    hasher.update("blob ".as_bytes());
    hasher.update(size.to_string().as_bytes());
    hasher.update([b'\0']);
    hasher.update(buffer.as_slice());
    let result = hasher.finalize();
    Ok(result.encode_hex::<String>())
}

fn to_glob(input: &str) -> Result<Glob, Error> {
    let glob = fix_glob_pattern(input).into_unix();
    let g = Glob::new(glob.as_str()).map(|g| g.into_owned())?;

    Ok(g)
}

pub(crate) fn hash_files(
    root_path: &AbsoluteSystemPath,
    files: impl Iterator<Item = impl AsRef<AnchoredSystemPath>>,
    allow_missing: bool,
) -> Result<GitHashes, Error> {
    let mut hashes = GitHashes::new();
    for file in files.into_iter() {
        let path = root_path.resolve(file.as_ref());
        match git_like_hash_file(&path) {
            Ok(hash) => hashes.insert(file.as_ref().to_unix(), hash),
            Err(Error::Io(ref io_error, _))
                if allow_missing && io_error.kind() == ErrorKind::NotFound =>
            {
                continue
            }
            Err(e) => return Err(e),
        };
    }
    Ok(hashes)
}

pub(crate) fn get_package_file_hashes_without_git<S: AsRef<str>>(
    turbo_root: &AbsoluteSystemPath,
    package_path: &AnchoredSystemPath,
    inputs: &[S],
    include_default_files: bool,
) -> Result<GitHashes, Error> {
    let full_package_path = turbo_root.resolve(package_path);
    let mut hashes = GitHashes::new();
    let mut default_file_hashes = GitHashes::new();
    let mut excluded_file_hashes = GitHashes::new();

    let mut walker_builder = WalkBuilder::new(&full_package_path);
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for pattern in inputs {
        let pattern = pattern.as_ref();
        if let Some(exclusion) = pattern.strip_prefix('!') {
            let g = to_glob(exclusion)?;
            excludes.push(g);
        } else {
            let g = to_glob(pattern)?;
            includes.push(g);
        }
    }
    let include_pattern = if includes.is_empty() {
        None
    } else {
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
        let turbo_g = to_glob("package.json")?;
        let package_g = to_glob("turbo.json")?;
        includes.push(turbo_g);
        includes.push(package_g);

        Some(any(includes)?)
    };
    let exclude_pattern = if excludes.is_empty() {
        None
    } else {
        Some(any(excludes)?)
    };

    let walker = walker_builder
        .follow_links(false)
        // if inputs have been provided manually, we shouldn't skip ignored files to mimic the
        // regular behavior
        .git_ignore(inputs.is_empty())
        .require_git(false)
        .hidden(false) // this results in yielding hidden files (e.g. .gitignore)
        .build();

    for dirent in walker {
        let dirent = dirent?;
        let metadata = dirent.metadata()?;
        // We need to do this here, rather than as a filter, because the root
        // directory is always yielded and not subject to the supplied filter.
        if metadata.is_dir() {
            continue;
        }

        let path = AbsoluteSystemPath::from_std_path(dirent.path())?;
        let relative_path = full_package_path.anchor(path)?;
        let relative_path = relative_path.to_unix();

        // if we have includes, and this path doesn't match any of them, skip it
        if let Some(include_pattern) = include_pattern.as_ref() {
            if !include_pattern.is_match(relative_path.as_str()) {
                continue;
            }
        }

        // if we have excludes, and this path matches one of them, skip it
        if let Some(exclude_pattern) = exclude_pattern.as_ref() {
            if exclude_pattern.is_match(relative_path.as_str()) {
                continue;
            }
        }

        // FIXME: we don't hash symlinks...
        if metadata.is_symlink() {
            continue;
        }
        let hash = git_like_hash_file(path)?;
        hashes.insert(relative_path, hash);
    }

    // If we're including default files, we need to walk again, but this time with
    // git_ignore enabled
    if include_default_files {
        let walker = walker_builder
            .follow_links(false)
            .git_ignore(true)
            .require_git(false)
            .hidden(false) // this results in yielding hidden files (e.g. .gitignore)
            .build();

        for dirent in walker {
            let dirent = dirent?;
            let metadata = dirent.metadata()?;
            // We need to do this here, rather than as a filter, because the root
            // directory is always yielded and not subject to the supplied filter.
            if metadata.is_dir() {
                continue;
            }

            let path = AbsoluteSystemPath::from_std_path(dirent.path())?;
            let relative_path = full_package_path.anchor(path)?;
            let relative_path = relative_path.to_unix();

            if let Some(exclude_pattern) = exclude_pattern.as_ref() {
                if exclude_pattern.is_match(relative_path.as_str()) {
                    // track excludes so we can exclude them to the hash map later
                    if !metadata.is_symlink() {
                        let hash = git_like_hash_file(path)?;
                        excluded_file_hashes.insert(relative_path.clone(), hash);
                    }
                }
            }

            // FIXME: we don't hash symlinks...
            if metadata.is_symlink() {
                continue;
            }
            let hash = git_like_hash_file(path)?;
            default_file_hashes.insert(relative_path, hash);
        }
    }

    // merge default with all hashes
    hashes.extend(default_file_hashes);
    // remove excluded files
    hashes.retain(|key, _| !excluded_file_hashes.contains_key(key));

    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use std::assert_matches::assert_matches;

    use test_case::test_case;
    use turbopath::{
        AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPath, RelativeUnixPathBuf,
    };

    use super::*;

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path().to_path_buf())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
    }

    #[test_case(&["non-existent-file.txt"], true, false ; "allow_missing, all missing")]
    #[test_case(&["non-existent-file.txt", "existing-file.txt"], true, false ; "allow_missing, some missing, some not")]
    #[test_case(&["existing-file.txt"], true, false ; "allow_missing, none missing")]
    #[test_case(&["non-existent-file.txt"], false, true ; "don't allow_missing, all missing")]
    #[test_case(&["non-existent-file.txt", "existing-file.txt"], false, true ; "don't allow_missing, some missing, some not")]
    #[test_case(&["existing-file.txt"], false, false ; "don't allow_missing, none missing")]
    fn test_hash_files(files: &[&str], allow_missing: bool, want_err: bool) {
        let (_tmp, turbo_root) = tmp_dir();
        let test_file = turbo_root.join_component("existing-file.txt");
        test_file.create_with_contents("").unwrap();

        let expected = {
            let mut expected = GitHashes::new();
            if files.contains(&"existing-file.txt") {
                expected.insert(
                    RelativeUnixPathBuf::new("existing-file.txt").unwrap(),
                    "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_string(),
                );
            }
            expected
        };

        let files = files
            .iter()
            .map(|s| AnchoredSystemPathBuf::from_raw(s).unwrap());
        match hash_files(&turbo_root, files, allow_missing) {
            Err(e) => assert!(want_err, "unexpected error {}", e),
            Ok(hashes) => assert_eq!(hashes, expected),
        }
    }

    #[test]
    fn test_hash_symlink() {
        let (_tmp, turbo_root) = tmp_dir();
        let from_to_file = turbo_root.join_component("symlink-from-to-file");
        let from_to_dir = turbo_root.join_component("symlink-from-to-dir");
        let broken = turbo_root.join_component("symlink-broken");

        let to_file = turbo_root.join_component("the-file-target");
        to_file.create_with_contents("contents").unwrap();

        let to_dir = turbo_root.join_component("the-dir-target");
        to_dir.create_dir_all().unwrap();

        from_to_file.symlink_to_file(to_file.to_string()).unwrap();
        from_to_dir.symlink_to_dir(to_dir.to_string()).unwrap();
        broken.symlink_to_file("does-not-exist").unwrap();

        // Symlink to file.
        let out = hash_files(
            &turbo_root,
            [AnchoredSystemPathBuf::from_raw("symlink-from-to-file").unwrap()].iter(),
            true,
        )
        .unwrap();
        let from_to_file_hash = out
            .get(&RelativeUnixPathBuf::new("symlink-from-to-file").unwrap())
            .unwrap();
        assert_eq!(
            from_to_file_hash,
            "0839b2e9412b314cb8bb9a20f587aa13752ae310"
        );

        // Symlink to dir, allow_missing = true.
        #[cfg(not(windows))]
        {
            let out = hash_files(
                &turbo_root,
                [AnchoredSystemPathBuf::from_raw("symlink-from-to-dir").unwrap()].iter(),
                true,
            );
            match out.err().unwrap() {
                Error::Io(io_error, _) => assert_eq!(io_error.kind(), ErrorKind::IsADirectory),
                _ => panic!("wrong error"),
            };
        }

        // Symlink to dir, allow_missing = false.
        let out = hash_files(
            &turbo_root,
            [AnchoredSystemPathBuf::from_raw("symlink-from-to-dir").unwrap()].iter(),
            false,
        );
        #[cfg(windows)]
        let expected_err_kind = ErrorKind::PermissionDenied;
        #[cfg(not(windows))]
        let expected_err_kind = ErrorKind::IsADirectory;
        assert_matches!(out.unwrap_err(), Error::Io(io_error, _) if io_error.kind() == expected_err_kind);

        // Broken symlink with allow_missing = true.
        let out = hash_files(
            &turbo_root,
            [AnchoredSystemPathBuf::from_raw("symlink-broken").unwrap()].iter(),
            true,
        )
        .unwrap();
        let broken_hash = out.get(&RelativeUnixPathBuf::new("symlink-broken").unwrap());
        assert_eq!(broken_hash, None);

        // Broken symlink with allow_missing = false.
        let out = hash_files(
            &turbo_root,
            [AnchoredSystemPathBuf::from_raw("symlink-broken").unwrap()].iter(),
            false,
        );
        match out.err().unwrap() {
            Error::Io(io_error, _) => assert_eq!(io_error.kind(), ErrorKind::NotFound),
            _ => panic!("wrong error"),
        };
    }

    #[test]
    fn test_get_package_file_hashes_from_processing_gitignore() {
        let root_ignore_contents = ["ignoreme", "ignorethisdir/"].join("\n");
        let pkg_ignore_contents = ["pkgignoreme", "pkgignorethisdir/"].join("\n");

        let (_tmp, turbo_root) = tmp_dir();

        let pkg_path = AnchoredSystemPathBuf::from_raw("child-dir/libA").unwrap();
        let unix_pkg_path = pkg_path.to_unix();
        let mut file_hash: Vec<(&str, &str, Option<&str>)> = vec![
            ("turbo.json", "turbo.json-file-contents", None),
            ("package.json", "root-package.json-file-contents", None),
            ("top-level-file", "top-level-file-contents", None),
            ("other-dir/other-dir-file", "other-dir-file-contents", None),
            ("ignoreme", "anything", None),
            (
                "child-dir/libA/turbo.json",
                "lib-turbo.json-content",
                Some("ca4dbb95c0829676756c6decae728252d4aa4911"),
            ),
            (
                "child-dir/libA/package.json",
                "lib-package.json-content",
                Some("55d57df9acc1b37d0cfc2c1c70379dab48f3f7e1"),
            ),
            (
                "child-dir/libA/some-file",
                "some-file-contents",
                Some("7e59c6a6ea9098c6d3beb00e753e2c54ea502311"),
            ),
            (
                "child-dir/libA/some-dir/other-file",
                "some-file-contents",
                Some("7e59c6a6ea9098c6d3beb00e753e2c54ea502311"),
            ),
            (
                "child-dir/libA/some-dir/another-one",
                "some-file-contents",
                Some("7e59c6a6ea9098c6d3beb00e753e2c54ea502311"),
            ),
            (
                "child-dir/libA/some-dir/excluded-file",
                "some-file-contents",
                Some("7e59c6a6ea9098c6d3beb00e753e2c54ea502311"),
            ),
            ("child-dir/libA/ignoreme", "anything", None),
            ("child-dir/libA/ignorethisdir/anything", "anything", None),
            ("child-dir/libA/pkgignoreme", "anything", None),
            ("child-dir/libA/pkgignorethisdir/file", "anything", None),
        ];

        let root_ignore_file = turbo_root.join_component(".gitignore");
        root_ignore_file
            .create_with_contents(root_ignore_contents)
            .unwrap();
        let pkg_ignore_file = turbo_root.resolve(&pkg_path).join_component(".gitignore");
        pkg_ignore_file.ensure_dir().unwrap();
        pkg_ignore_file
            .create_with_contents(pkg_ignore_contents)
            .unwrap();

        let mut expected = GitHashes::new();
        for (raw_unix_path, contents, expected_hash) in file_hash.iter() {
            let unix_path = RelativeUnixPath::new(raw_unix_path).unwrap();
            let file_path = turbo_root.join_unix_path(unix_path).unwrap();
            file_path.ensure_dir().unwrap();
            file_path.create_with_contents(contents).unwrap();
            if let Some(hash) = expected_hash {
                println!("unix_path: {}", unix_path);
                println!("unix_pkg_path: {}", unix_pkg_path);
                let unix_pkg_file_path = unix_path.strip_prefix(&unix_pkg_path).unwrap();
                println!("unix_pkg_file_path: {}", unix_pkg_file_path);
                expected.insert(unix_pkg_file_path.to_owned(), (*hash).to_owned());
            }
        }
        expected.insert(
            RelativeUnixPathBuf::new(".gitignore").unwrap(),
            "3237694bc3312ded18386964a855074af7b066af".to_owned(),
        );

        let hashes =
            get_package_file_hashes_without_git::<&str>(&turbo_root, &pkg_path, &[], false)
                .unwrap();
        assert_eq!(hashes, expected);

        // set a hash for an ignored file
        for (raw_unix_path, _, expected_hash) in file_hash.iter_mut() {
            if *raw_unix_path == "child-dir/libA/pkgignorethisdir/file" {
                *expected_hash = Some("67aed78ea231bdee3de45b6d47d8f32a0a792f6d");
                break;
            }
        }

        expected = GitHashes::new();
        for (raw_unix_path, contents, expected_hash) in file_hash.iter() {
            let unix_path = RelativeUnixPath::new(raw_unix_path).unwrap();
            let file_path = turbo_root.join_unix_path(unix_path).unwrap();
            file_path.ensure_dir().unwrap();
            file_path.create_with_contents(contents).unwrap();
            if let Some(hash) = expected_hash {
                let unix_pkg_file_path = unix_path.strip_prefix(&unix_pkg_path).unwrap();
                if (unix_pkg_file_path.ends_with("file")
                    || unix_pkg_file_path.ends_with("package.json")
                    || unix_pkg_file_path.ends_with("turbo.json"))
                    && !unix_pkg_file_path.ends_with("excluded-file")
                {
                    expected.insert(unix_pkg_file_path.to_owned(), (*hash).to_owned());
                }
            }
        }

        let hashes = get_package_file_hashes_without_git(
            &turbo_root,
            &pkg_path,
            &["**/*file", "!some-dir/excluded-file"],
            false,
        )
        .unwrap();

        assert_eq!(hashes, expected);
    }
}
