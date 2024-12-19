#![cfg(feature = "walk")]

use std::{collections::HashSet, path::PathBuf};

use build_fs_tree::{dir, file, Build, FileSystemTree};
use tempfile::TempDir;
use wax::{
    walk::{Entry, FileIterator, WalkBehavior},
    Glob,
};

// TODO: Rust's testing framework does not provide a mechanism for maintaining
//       shared state. This means that tests that write to the file system must
//       do so individually rather than writing before and after all tests have
//       run. This should probably be avoided.

/// Writes a testing directory tree to a temporary location on the file system.
fn temptree() -> (TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let tree: FileSystemTree<&str, &str> = dir! {
        "doc" => dir! {
            "guide.md" => file!(""),
        },
        "src" => dir! {
            "glob.rs" => file!(""),
            "lib.rs" => file!(""),
        },
        "tests" => dir! {
            "walk.rs" => file!(""),
        },
        "README.md" => file!(""),
    };
    let path = root.path().join("project");
    tree.build(&path).unwrap();
    (root, path)
}

/// Writes a testing directory tree that includes a reentrant symbolic link to a
/// temporary location on the file system.
#[cfg(any(unix, windows))]
fn temptree_with_cyclic_link() -> (TempDir, PathBuf) {
    use std::{io, path::Path};

    #[cfg(unix)]
    fn link(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn link(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    // Get a temporary tree and create a reentrant symbolic link.
    let (root, path) = temptree();
    link(path.as_path(), path.join("tests/cycle")).unwrap();
    (root, path)
}

#[test]
fn walk_with_tree() {
    let (_root, path) = temptree();

    let glob = Glob::new("**").unwrap();
    let paths: HashSet<_> = glob
        .walk(&path)
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(
        paths,
        [
            #[allow(clippy::redundant_clone)]
            path.to_path_buf(),
            path.join("doc"),
            path.join("doc/guide.md"),
            path.join("src"),
            path.join("src/glob.rs"),
            path.join("src/lib.rs"),
            path.join("tests"),
            path.join("tests/walk.rs"),
            path.join("README.md"),
        ]
        .into_iter()
        .collect(),
    );
}

#[test]
fn walk_with_invariant_terminating_component() {
    let (_root, path) = temptree();

    let glob = Glob::new("**/*.md").unwrap();
    let paths: HashSet<_> = glob
        .walk(&path)
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(
        paths,
        IntoIterator::into_iter([path.join("doc/guide.md"), path.join("README.md"),]).collect(),
    );
}

#[test]
fn walk_with_invariant_intermediate_component() {
    let (_root, path) = temptree();

    let glob = Glob::new("**/src/**/*.rs").unwrap();
    let paths: HashSet<_> = glob
        .walk(&path)
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(
        paths,
        IntoIterator::into_iter([path.join("src/glob.rs"), path.join("src/lib.rs"),]).collect(),
    );
}

#[test]
fn walk_with_invariant_glob() {
    let (_root, path) = temptree();

    let glob = Glob::new("src/lib.rs").unwrap();
    let paths: HashSet<_> = glob
        .walk(&path)
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(paths, [path.join("src/lib.rs")].into_iter().collect(),);
}

#[test]
fn walk_with_invariant_partitioned_glob() {
    let (_root, path) = temptree();

    let (prefix, glob) = Glob::new("src/lib.rs").unwrap().partition();
    let paths: HashSet<_> = glob
        .walk(path.join(prefix))
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(paths, [path.join("src/lib.rs")].into_iter().collect(),);
}

#[test]
fn walk_with_not() {
    let (_root, path) = temptree();

    let glob = Glob::new("**/*.{md,rs}").unwrap();
    let paths: HashSet<_> = glob
        .walk(&path)
        .not(["tests/**"])
        .unwrap()
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(
        paths,
        [
            path.join("doc/guide.md"),
            path.join("src/glob.rs"),
            path.join("src/lib.rs"),
            path.join("README.md"),
        ]
        .into_iter()
        .collect(),
    );
}

#[test]
fn walk_with_depth() {
    let (_root, path) = temptree();

    let glob = Glob::new("**").unwrap();
    let paths: HashSet<_> = glob
        .walk_with_behavior(
            &path,
            WalkBehavior {
                depth: 1,
                ..Default::default()
            },
        )
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(
        paths,
        [
            #[allow(clippy::redundant_clone)]
            path.to_path_buf(),
            path.join("doc"),
            path.join("src"),
            path.join("tests"),
            path.join("README.md"),
        ]
        .into_iter()
        .collect(),
    );
}

#[test]
#[cfg(any(unix, windows))]
fn walk_with_cyclic_link_file() {
    use wax::walk::LinkBehavior;

    let (_root, path) = temptree_with_cyclic_link();

    let glob = Glob::new("**").unwrap();
    let paths: HashSet<_> = glob
        .walk_with_behavior(&path, LinkBehavior::ReadFile)
        .flatten()
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(
        paths,
        [
            #[allow(clippy::redundant_clone)]
            path.to_path_buf(),
            path.join("README.md"),
            path.join("doc"),
            path.join("doc/guide.md"),
            path.join("src"),
            path.join("src/glob.rs"),
            path.join("src/lib.rs"),
            path.join("tests"),
            path.join("tests/cycle"),
            path.join("tests/walk.rs"),
        ]
        .into_iter()
        .collect(),
    );
}

#[test]
#[cfg(any(unix, windows))]
fn walk_with_cyclic_link_target() {
    use wax::walk::LinkBehavior;

    let (_root, path) = temptree_with_cyclic_link();

    // Collect paths into `Vec`s so that duplicates can be detected.
    let expected = vec![
        #[allow(clippy::redundant_clone)]
        path.to_path_buf(),
        path.join("README.md"),
        path.join("doc"),
        path.join("doc/guide.md"),
        path.join("src"),
        path.join("src/glob.rs"),
        path.join("src/lib.rs"),
        path.join("tests"),
        path.join("tests/walk.rs"),
    ];
    let glob = Glob::new("**").unwrap();
    let mut paths: Vec<_> = glob
        .walk_with_behavior(&path, LinkBehavior::ReadTarget)
        .flatten()
        // Take an additional item. This prevents an infinite loop if there is a
        // problem with detecting the cycle while also introducing unexpected
        // files so that the error can be detected.
        .take(expected.len() + 1)
        .map(|entry| entry.into_path())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, expected);
}
