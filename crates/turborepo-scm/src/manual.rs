use std::{fs::Metadata, io::Read};

use globwalk::fix_glob_pattern;
use hex::ToHex;
use ignore::WalkBuilder;
use sha1::{Digest, Sha1};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, IntoUnix};
use wax::{any, Glob, Pattern};

use crate::{package_deps::GitHashes, Error};

fn git_like_hash_file(path: &AbsoluteSystemPath, metadata: &Metadata) -> Result<String, Error> {
    let mut hasher = Sha1::new();
    let mut f = path.open()?;
    let mut buffer = Vec::new();
    f.read_to_end(&mut buffer)?;
    hasher.update("blob ".as_bytes());
    hasher.update(metadata.len().to_string().as_bytes());
    hasher.update([b'\0']);
    hasher.update(buffer.as_slice());
    let result = hasher.finalize();
    Ok(result.encode_hex::<String>())
}

pub(crate) fn hash_files(
    root_path: &AbsoluteSystemPath,
    files: impl Iterator<Item = AnchoredSystemPathBuf>,
    allow_missing: bool,
) -> Result<GitHashes, Error> {
    let mut hashes = GitHashes::new();
    for file in files.into_iter() {
        let path = root_path.resolve(&file);
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(e) => {
                if allow_missing && e.is_io_error(std::io::ErrorKind::NotFound) {
                    continue;
                }
                return Err(e.into());
            }
        };
        let hash = git_like_hash_file(&path, &metadata)?;
        hashes.insert(file.to_unix(), hash);
    }
    Ok(hashes)
}

pub(crate) fn get_package_file_hashes_from_processing_gitignore<S: AsRef<str>>(
    turbo_root: &AbsoluteSystemPath,
    package_path: &AnchoredSystemPathBuf,
    inputs: &[S],
) -> Result<GitHashes, Error> {
    let full_package_path = turbo_root.resolve(package_path);
    let mut hashes = GitHashes::new();

    let mut walker_builder = WalkBuilder::new(&full_package_path);
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for pattern in inputs {
        let pattern = pattern.as_ref();
        if let Some(exclusion) = pattern.strip_prefix('!') {
            let glob = fix_glob_pattern(exclusion).into_unix();
            let g = Glob::new(glob.as_str()).map(|g| g.into_owned())?;
            excludes.push(g);
        } else {
            let glob = fix_glob_pattern(pattern).into_unix();
            let g = Glob::new(glob.as_str()).map(|g| g.into_owned())?;
            includes.push(g);
        }
    }
    let include_pattern = if includes.is_empty() {
        None
    } else {
        Some(any(includes)?)
    };
    let exclude_pattern = if excludes.is_empty() {
        None
    } else {
        Some(any(excludes)?)
    };
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
        if let Some(include_pattern) = include_pattern.as_ref() {
            if !include_pattern.is_match(relative_path.as_str()) {
                continue;
            }
        }
        if let Some(exclude_pattern) = exclude_pattern.as_ref() {
            if exclude_pattern.is_match(relative_path.as_str()) {
                continue;
            }
        }
        // FIXME: we don't hash symlinks...
        if metadata.is_symlink() {
            continue;
        }
        let hash = git_like_hash_file(path, &metadata)?;
        hashes.insert(relative_path, hash);
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, RelativeUnixPath, RelativeUnixPathBuf};

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
    fn test_get_package_file_hashes_from_processing_gitignore() {
        let root_ignore_contents = ["ignoreme", "ignorethisdir/"].join("\n");
        let pkg_ignore_contents = ["pkgignoreme", "pkgignorethisdir/"].join("\n");

        let (_tmp, turbo_root) = tmp_dir();

        let pkg_path = AnchoredSystemPathBuf::from_raw("child-dir/libA").unwrap();
        let unix_pkg_path = pkg_path.to_unix().unwrap();
        let file_hash: Vec<(&str, &str, Option<&str>)> = vec![
            ("top-level-file", "top-level-file-contents", None),
            ("other-dir/other-dir-file", "other-dir-file-contents", None),
            ("ignoreme", "anything", None),
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
            .create_with_contents(&root_ignore_contents)
            .unwrap();
        let pkg_ignore_file = turbo_root.resolve(&pkg_path).join_component(".gitignore");
        pkg_ignore_file.ensure_dir().unwrap();
        pkg_ignore_file
            .create_with_contents(&pkg_ignore_contents)
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
            get_package_file_hashes_from_processing_gitignore::<&str>(&turbo_root, &pkg_path, &[])
                .unwrap();
        assert_eq!(hashes, expected);

        expected = GitHashes::new();
        for (raw_unix_path, contents, expected_hash) in file_hash.iter() {
            let unix_path = RelativeUnixPath::new(raw_unix_path).unwrap();
            let file_path = turbo_root.join_unix_path(unix_path).unwrap();
            file_path.ensure_dir().unwrap();
            file_path.create_with_contents(contents).unwrap();
            if let Some(hash) = expected_hash {
                let unix_pkg_file_path = unix_path.strip_prefix(&unix_pkg_path).unwrap();
                if unix_pkg_file_path.ends_with("file")
                    && !unix_pkg_file_path.ends_with("excluded-file")
                {
                    expected.insert(unix_pkg_file_path.to_owned(), (*hash).to_owned());
                }
            }
        }

        let hashes = get_package_file_hashes_from_processing_gitignore(
            &turbo_root,
            &pkg_path,
            &["**/*file", "!some-dir/excluded-file"],
        )
        .unwrap();
        assert_eq!(hashes, expected);
    }
}
