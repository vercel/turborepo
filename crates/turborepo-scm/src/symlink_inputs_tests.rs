#![cfg(any(unix, windows))]

#[cfg(unix)]
use std::os::unix::fs::symlink;

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{GitHashes, SCM, test_utils};

#[cfg(windows)]
fn symlink(
    target: impl AsRef<std::path::Path>,
    link: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    let target: std::path::PathBuf = target.as_ref().components().collect();
    let link = link.as_ref();
    if link.parent().unwrap().join(&target).is_dir() {
        std::os::windows::fs::symlink_dir(&target, link)
    } else {
        std::os::windows::fs::symlink_file(&target, link)
    }
}

fn remove_link(path: impl AsRef<std::path::Path>) {
    let path = path.as_ref();
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if std::fs::symlink_metadata(path)
            .unwrap()
            .file_type()
            .is_symlink_dir()
        {
            std::fs::remove_dir(path).unwrap();
            return;
        }
    }
    std::fs::remove_file(path).unwrap();
}

fn hashes(
    root: &AbsoluteSystemPath,
    inputs: &[&str],
    backend: usize,
) -> Result<GitHashes, crate::Error> {
    hashes_with_defaults(root, inputs, backend, false)
}

fn hashes_with_defaults(
    root: &AbsoluteSystemPath,
    inputs: &[&str],
    backend: usize,
    include_defaults: bool,
) -> Result<GitHashes, crate::Error> {
    let scm = if backend == 2 {
        SCM::Manual
    } else {
        SCM::new(root)
    };
    let index = (backend == 1).then(|| scm.build_repo_index_eager().unwrap());
    scm.get_package_file_hashes(
        root,
        &AnchoredSystemPathBuf::from_raw("packages/a").unwrap(),
        inputs,
        include_defaults,
        None,
        index.as_ref(),
    )
}

#[test]
fn tracked_ignored_link_names_are_literal_inputs() {
    for name in ["plain", "name[1]", "name{one,two}", "name($)"] {
        for directory in [false, true] {
            let (_tmp, root) = test_utils::tmp_dir();
            test_utils::init_repo(&root);
            let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
            at("packages/a/src").create_dir_all().unwrap();
            at("shared/one").create_dir_all().unwrap();
            at("shared/two").create_dir_all().unwrap();
            for target in ["one", "two"] {
                at(&format!("shared/{target}/child.txt"))
                    .create_with_contents("V1")
                    .unwrap();
                at(&format!("shared/{target}/skip.txt"))
                    .create_with_contents("excluded")
                    .unwrap();
            }
            let logical = format!("src/{name}");
            let link = at(&format!("packages/a/{logical}"));
            let target = |name| {
                format!(
                    "../../../shared/{name}{}",
                    if directory { "" } else { "/child.txt" }
                )
            };
            symlink(target("one"), &link).unwrap();
            assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
            test_utils::commit_all(&root);
            at(".gitignore")
                .create_with_contents("packages/a/src/\n")
                .unwrap();
            test_utils::commit_all(&root);
            let key = RelativeUnixPathBuf::new(if directory {
                format!("{logical}/child.txt")
            } else {
                logical.clone()
            })
            .unwrap();
            for mode in 0..2 {
                for include_defaults in [false, true] {
                    let inputs: &[&str] = if include_defaults {
                        &["!src/**/skip.txt"]
                    } else {
                        &[]
                    };
                    let capture =
                        || hashes_with_defaults(&root, inputs, mode, include_defaults).unwrap();
                    let before = capture();
                    assert!(
                        before.contains_key(&key),
                        "{name} directory={directory} backend={mode}"
                    );
                    assert_eq!(before, capture());
                    if directory && include_defaults {
                        assert!(!before.contains_key(
                            &RelativeUnixPathBuf::new(format!("{logical}/skip.txt")).unwrap()
                        ));
                    }
                    at("shared/one/child.txt")
                        .create_with_contents("V2")
                        .unwrap();
                    let edited = capture();
                    assert_ne!(before[&key], edited[&key]);
                    assert_eq!(edited, capture());
                    at("shared/two/child.txt")
                        .create_with_contents("V2")
                        .unwrap();
                    remove_link(&link);
                    symlink(target("two"), &link).unwrap();
                    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
                    // Keep this a tracked-inventory test. Unstaged ignored
                    // directory retargets can disappear from the Git inventory.
                    test_utils::commit_all(&root);
                    let retargeted = capture();
                    assert!(
                        retargeted.contains_key(&key),
                        "retargeted input missing: {name} directory={directory} backend={mode}"
                    );
                    assert_ne!(edited, retargeted, "identical-content retarget: {name}");
                    assert_eq!(retargeted, capture());
                    at("shared/two/child.txt")
                        .create_with_contents("V3")
                        .unwrap();
                    let retargeted_edit = capture();
                    assert_ne!(retargeted[&key], retargeted_edit[&key]);
                    assert_eq!(retargeted_edit, capture());
                    remove_link(&link);
                    symlink(target("one"), &link).unwrap();
                    at("shared/one/child.txt")
                        .create_with_contents("V1")
                        .unwrap();
                    at("shared/two/child.txt")
                        .create_with_contents("V1")
                        .unwrap();
                    test_utils::commit_all(&root);
                    assert_eq!(before, capture());
                }
            }
        }
    }
}

#[test]
fn unstaged_directory_retarget_retains_ignored_tracked_inputs() {
    for name in ["dir", "dir[1]"] {
        for mode in 0..2 {
            for include_defaults in [false, true] {
                let (_tmp, root) = test_utils::tmp_dir();
                test_utils::init_repo(&root);
                let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
                at("packages/a/src").create_dir_all().unwrap();
                for target in ["one", "two"] {
                    at(&format!("shared/{target}")).create_dir_all().unwrap();
                    at(&format!("shared/{target}/child.txt"))
                        .create_with_contents("V1")
                        .unwrap();
                    at(&format!("shared/{target}/skip.txt"))
                        .create_with_contents("ignored")
                        .unwrap();
                }
                let logical = format!("src/{name}");
                let link = at(&format!("packages/a/{logical}"));
                symlink("../../../shared/one", &link).unwrap();
                assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
                test_utils::commit_all(&root);
                at(".gitignore")
                    .create_with_contents("packages/a/src/\n")
                    .unwrap();
                test_utils::commit_all(&root);
                let inputs: &[&str] = if include_defaults {
                    &["!src/**/skip.txt"]
                } else {
                    &[]
                };
                let capture =
                    || hashes_with_defaults(&root, inputs, mode, include_defaults).unwrap();
                let directory_key = RelativeUnixPathBuf::new(&logical).unwrap();
                let child_key = RelativeUnixPathBuf::new(format!("{logical}/child.txt")).unwrap();
                let before = capture();
                assert!(before.contains_key(&directory_key) && before.contains_key(&child_key));
                assert_eq!(before, capture());
                remove_link(&link);
                symlink("../../../shared/two", &link).unwrap();
                assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
                // Deliberately do not stage or commit the new link target.
                let retargeted = capture();
                assert!(
                    retargeted.contains_key(&directory_key),
                    "lost directory: {name} backend={mode}"
                );
                assert!(
                    retargeted.contains_key(&child_key),
                    "lost child: {name} backend={mode}"
                );
                assert_ne!(before[&directory_key], retargeted[&directory_key]);
                assert_eq!(retargeted, capture());
                at("shared/two/child.txt")
                    .create_with_contents("V2")
                    .unwrap();
                let edited = capture();
                assert_ne!(retargeted[&child_key], edited[&child_key]);
                assert_eq!(edited, capture());
                if include_defaults {
                    let skip_key = RelativeUnixPathBuf::new(format!("{logical}/skip.txt")).unwrap();
                    assert!(!edited.contains_key(&skip_key));
                    at("shared/two/skip.txt")
                        .create_with_contents("still ignored")
                        .unwrap();
                    assert_eq!(edited, capture());
                }
                at("shared/one/child.txt")
                    .create_with_contents("not selected")
                    .unwrap();
                assert_eq!(edited, capture());
                test_utils::commit_all(&root);
                assert_eq!(
                    edited,
                    capture(),
                    "staging/committing unchanged inputs changed the hash"
                );
            }
        }
    }
}

#[test]
fn unstaged_dangling_retargets_retain_ignored_tracked_inputs() {
    for name in ["link", "link[1]"] {
        for directory in [false, true] {
            for mode in 0..2 {
                for include_defaults in [false, true] {
                    let (_tmp, root) = test_utils::tmp_dir();
                    test_utils::init_repo(&root);
                    let at =
                        |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
                    at("packages/a/src").create_dir_all().unwrap();
                    at("shared").create_dir_all().unwrap();
                    let logical = format!("src/{name}");
                    let link = at(&format!("packages/a/{logical}"));
                    let retarget = |target: &str| {
                        // Create the target briefly so Windows creates the right
                        // link kind, then leave a real dangling link behind.
                        let target_path = at(&format!("shared/{target}"));
                        if directory {
                            target_path.create_dir_all().unwrap();
                        } else {
                            target_path.create_with_contents("temporary").unwrap();
                        }
                        symlink(format!("../../../shared/{target}"), &link).unwrap();
                        if directory {
                            std::fs::remove_dir(target_path).unwrap();
                        } else {
                            std::fs::remove_file(target_path).unwrap();
                        }
                        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
                        assert!(!link.as_std_path().exists());
                    };
                    retarget("one");
                    test_utils::commit_all(&root);
                    at(".gitignore")
                        .create_with_contents("packages/a/src/\n")
                        .unwrap();
                    test_utils::commit_all(&root);
                    let inputs: &[&str] = if include_defaults {
                        &["!src/excluded/**"]
                    } else {
                        &[]
                    };
                    let capture =
                        || hashes_with_defaults(&root, inputs, mode, include_defaults).unwrap();
                    let key = RelativeUnixPathBuf::new(logical).unwrap();
                    let mut previous = capture();
                    assert!(previous.contains_key(&key));
                    assert_eq!(previous, capture());
                    assert!(
                        SCM::new(&root)
                            .get_hashes_for_files(
                                &root,
                                &[AnchoredSystemPathBuf::from_raw(format!(
                                    "packages/a/src/{name}"
                                ))
                                .unwrap()],
                                false,
                            )
                            .is_err(),
                        "strict file hashing must still reject a missing referent"
                    );
                    for target in ["two", "three"] {
                        remove_link(&link);
                        retarget(target);
                        // Both changes remain unstaged. Losing the inventory
                        // entry can hide the second retarget behind a cache hit.
                        let changed = capture();
                        assert!(
                            changed.contains_key(&key),
                            "lost dangling link: {name} directory={directory} backend={mode} \
                             defaults={include_defaults}"
                        );
                        assert_ne!(previous[&key], changed[&key]);
                        assert_eq!(changed, capture());
                        previous = changed;
                    }
                    test_utils::commit_all(&root);
                    assert_eq!(previous, capture());
                    assert!(
                        !hashes_with_defaults(&root, &["!src/**"], mode, true)
                            .unwrap()
                            .contains_key(&key),
                        "excluded dangling links must remain excluded"
                    );
                    remove_link(&link);
                    assert!(!capture().contains_key(&key));
                }
            }
        }
    }
}

#[test]
fn authored_link_globs_remain_patterns() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("shared").create_dir_all().unwrap();
    at("shared/file.txt").create_with_contents("V1").unwrap();
    for name in ["file1.txt", "file2.txt", "file[1].txt"] {
        let link = at(&format!("packages/a/src/{name}"));
        symlink("../../../shared/file.txt", &link).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    }
    test_utils::commit_all(&root);
    at(".gitignore")
        .create_with_contents("packages/a/src/\n")
        .unwrap();
    test_utils::commit_all(&root);
    for mode in 0..2 {
        let selected = hashes(&root, &["src/file[12].txt", "!src/file2.txt"], mode).unwrap();
        let names: Vec<_> = selected.keys().map(|path| path.as_str()).collect();
        assert_eq!(names, ["src/file1.txt"]);
        assert_eq!(
            selected,
            hashes(&root, &["src/file[12].txt", "!src/file2.txt"], mode).unwrap()
        );
    }
}

#[test]
fn symlink_contents_invalidate_inputs() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("shared/dir").create_dir_all().unwrap();
    at("shared/x.txt").create_with_contents("V1").unwrap();
    at("shared/dir/y.txt").create_with_contents("D1").unwrap();
    at("shared/dir/skip.txt")
        .create_with_contents("ignored")
        .unwrap();
    symlink("../../../shared/x.txt", at("packages/a/src/x.txt")).unwrap();
    symlink("../../../shared/dir", at("packages/a/src/dir")).unwrap();
    test_utils::commit_all(&root);
    let inputs = ["src/**", "!src/dir/skip.txt"];
    let before: Vec<_> = (0..3)
        .map(|mode| hashes(&root, &inputs, mode).unwrap())
        .collect();
    for (mode, result) in before.iter().enumerate() {
        let mut names: Vec<_> = result.keys().map(|path| path.as_str()).collect();
        names.sort();
        assert_eq!(names, ["src/dir", "src/dir/y.txt", "src/x.txt"]);
        assert_eq!(result, &before[0]);
        assert!(
            hashes(&root, &[], mode)
                .unwrap()
                .contains_key(&RelativeUnixPathBuf::new("src/dir/y.txt").unwrap())
        );
        let literal = hashes(&root, &["src/dir"], mode).unwrap();
        for (path, hash) in hashes(&root, &["src/dir/**/*.txt"], mode).unwrap() {
            assert_eq!(literal.get(&path), Some(&hash));
        }
    }
    at("shared/x.txt").create_with_contents("V2").unwrap();
    at("shared/dir/y.txt").create_with_contents("D2").unwrap();
    for (mode, old) in before.iter().enumerate() {
        let new = hashes(&root, &inputs, mode).unwrap();
        for key in ["src/dir/y.txt", "src/x.txt"] {
            let key = RelativeUnixPathBuf::new(key).unwrap();
            assert_ne!(old[&key], new[&key]);
        }
    }
    let dirty: Vec<_> = (0..3)
        .map(|mode| hashes(&root, &inputs, mode).unwrap())
        .collect();
    test_utils::commit_all(&root);
    for (mode, expected) in dirty.iter().enumerate() {
        assert_eq!(hashes(&root, &inputs, mode).unwrap(), *expected);
    }
    at("shared/dir/skip.txt")
        .create_with_contents("changed excluded file")
        .unwrap();
    for (mode, expected) in dirty.iter().enumerate() {
        assert_eq!(hashes(&root, &inputs, mode).unwrap(), *expected);
    }
}

#[test]
fn symlink_inputs_reject_external_and_git_metadata_targets() {
    let (_tmp, root) = test_utils::tmp_dir();
    let (_outside_tmp, outside) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    root.join_components(&["packages", "a", "src"])
        .create_dir_all()
        .unwrap();
    root.join_components(&["packages", "a", "src", "control.txt"])
        .create_with_contents("safe")
        .unwrap();
    test_utils::commit_all(&root);
    outside
        .join_component("file.txt")
        .create_with_contents("outside")
        .unwrap();
    let link = root.join_components(&["packages", "a", "src", "link"]);
    for target in [
        outside.join_component("file.txt"),
        outside,
        root.join_components(&[".git", "config"]),
        root.join_component(".git"),
    ] {
        symlink(&target, &link).unwrap();
        for mode in 0..3 {
            assert!(
                hashes(&root, &["src/link"], mode).is_err(),
                "backend {mode} accepted {target}"
            );
        }
        remove_link(&link);
    }
}

#[test]
fn excluded_dirty_link_keeps_tracked_defaults() {
    let (_tmp, root) = test_utils::tmp_dir();
    let (_outside_tmp, outside) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let package = AnchoredSystemPathBuf::from_raw("packages/a").unwrap();
    let dir = root.resolve(&package);
    dir.join_component("src").create_dir_all().unwrap();
    root.join_component(".gitignore")
        .create_with_contents("packages/a/control.txt\n")
        .unwrap();
    dir.join_component("control.txt")
        .create_with_contents("tracked")
        .unwrap();
    root.join_component("target.txt")
        .create_with_contents("inside")
        .unwrap();
    let link = dir.join_components(&["src", "excluded"]);
    symlink("../../../target.txt", &link).unwrap();
    test_utils::require_git_cmd(&root, &["add", "-f", "packages/a/control.txt"]);
    test_utils::commit_all(&root);
    outside
        .join_component("target.txt")
        .create_with_contents("outside")
        .unwrap();
    remove_link(&link);
    symlink(outside.join_component("target.txt"), &link).unwrap();
    let scm = SCM::new(&root);
    let index = scm.build_repo_index_eager().unwrap();
    for index in [None, Some(&index)] {
        let result = scm
            .get_package_file_hashes(
                &root,
                &package,
                &["src/**", "!src/excluded"],
                true,
                None,
                index,
            )
            .unwrap();
        assert!(result.contains_key(&RelativeUnixPathBuf::new("control.txt").unwrap()));
    }
}

#[test]
fn excluded_and_unmatched_symlink_cycles() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let src = root.join_components(&["packages", "a", "src"]);
    src.create_dir_all().unwrap();
    src.join_component("a.txt")
        .create_with_contents("safe")
        .unwrap();
    symlink(".", src.join_component("ignored-cycle")).unwrap();
    test_utils::commit_all(&root);
    for inputs in [
        vec!["src/*.txt"],
        vec!["src/**", "!src/ignored-cycle/**"],
        vec!["src/**", "!src/{ignored-cycle,other}/**"],
    ] {
        for mode in 0..3 {
            let actual = hashes(&root, &inputs, mode).unwrap();
            assert_eq!(actual.len(), 1, "{inputs:?}, backend {mode}");
            assert!(actual.contains_key(&RelativeUnixPathBuf::new("src/a.txt").unwrap()));
            assert_eq!(actual, hashes(&root, &inputs, mode).unwrap());
        }
    }
    for mode in 0..3 {
        assert!(
            hashes(&root, &["src/**"], mode).is_err(),
            "an included cycle must still fail"
        );
    }
}

#[test]
fn parent_symlink_reaches_selected_sibling() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    root.join_components(&["packages", "a", "src"])
        .create_dir_all()
        .unwrap();
    root.join_component("shared").create_dir_all().unwrap();
    let file = root.join_components(&["shared", "value.txt"]);
    file.create_with_contents("before").unwrap();
    symlink(
        "../../..",
        root.join_components(&["packages", "a", "src", "parent"]),
    )
    .unwrap();
    test_utils::commit_all(&root);
    let inputs = ["src/parent/shared/**/*.txt"];
    let before: Vec<_> = (0..3)
        .map(|mode| hashes(&root, &inputs, mode).unwrap())
        .collect();
    assert_eq!(before[0], before[1]);
    assert_eq!(before[0], before[2]);
    file.create_with_contents("after").unwrap();
    for (mode, old) in before.iter().enumerate() {
        let actual = hashes(&root, &inputs, mode).unwrap();
        assert_ne!(&actual, old);
        assert_eq!(actual, hashes(&root, &inputs, mode).unwrap());
    }
}

#[test]
fn identical_content_retargeting_preserves_link_identity() {
    for kind in ["file", "directory", "empty", "broken"] {
        let (_tmp, root) = test_utils::tmp_dir();
        test_utils::init_repo(&root);
        let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
        at("packages/a/src").create_dir_all().unwrap();
        for target in ["one", "two"] {
            match kind {
                "file" => at(target).create_with_contents("same bytes").unwrap(),
                "directory" => {
                    at(target).create_dir_all().unwrap();
                    at(&format!("{target}/child.txt"))
                        .create_with_contents("same bytes")
                        .unwrap();
                }
                "empty" => at(target).create_dir_all().unwrap(),
                "broken" => {}
                _ => unreachable!(),
            }
        }
        let link = at("packages/a/src/link");
        symlink("../../../one", &link).unwrap();
        test_utils::commit_all(&root);
        let mut selectors = vec![vec!["src/**"], vec!["src/link"]];
        if kind == "directory" {
            selectors.push(vec!["src/*/child.txt"]);
        }
        let before: Vec<Vec<_>> = selectors
            .iter()
            .map(|inputs| {
                (0..3)
                    .map(|mode| hashes(&root, inputs, mode).unwrap())
                    .collect()
            })
            .collect();
        for modes in &before {
            assert!(!modes[0].is_empty(), "{kind} link must remain an input");
            assert_eq!(modes[0], modes[1]);
            assert_eq!(modes[0], modes[2]);
        }
        remove_link(&link);
        symlink("../../../two", &link).unwrap();
        for (selector, old) in selectors.iter().zip(&before) {
            for (mode, expected) in old.iter().enumerate() {
                let actual = hashes(&root, selector, mode).unwrap();
                assert_ne!(&actual, expected, "{kind}, {selector:?}, backend {mode}");
                assert_eq!(actual, hashes(&root, selector, mode).unwrap());
            }
        }
        let dirty: Vec<_> = selectors
            .iter()
            .map(|inputs| {
                (0..3)
                    .map(|mode| hashes(&root, inputs, mode).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect();
        test_utils::commit_all(&root);
        for (selector, expected) in selectors.iter().zip(dirty) {
            for (mode, expected) in expected.into_iter().enumerate() {
                assert_eq!(
                    hashes(&root, selector, mode).unwrap(),
                    expected,
                    "committing an unchanged link must not affect its hash"
                );
            }
        }
    }
}

#[test]
fn excluded_cycles_also_preserve_default_inputs() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let src = root.join_components(&["packages", "a", "src"]);
    src.create_dir_all().unwrap();
    src.join_component("a.txt")
        .create_with_contents("safe")
        .unwrap();
    root.join_components(&["packages", "a", "control.txt"])
        .create_with_contents("default input")
        .unwrap();
    symlink(".", src.join_component("ignored-cycle")).unwrap();
    test_utils::commit_all(&root);
    for inputs in [
        vec!["src/**", "!src/ignored-cycle/**"],
        vec!["src/**", "!../../packages/a/src/ignored-cycle/**"],
        vec!["!src/ignored-cycle/**"],
        vec!["src/**", "!src/ignored-cycle/**/"],
        vec!["src/**", "!src/{ignored-cycle,other}/**"],
    ] {
        let mut previous = None;
        for mode in 0..3 {
            let actual = hashes_with_defaults(&root, &inputs, mode, true)
                .unwrap_or_else(|error| panic!("{inputs:?}, backend {mode}: {error:?}"));
            assert!(actual.contains_key(&RelativeUnixPathBuf::new("control.txt").unwrap()));
            assert!(actual.contains_key(&RelativeUnixPathBuf::new("src/a.txt").unwrap()));
            assert!(
                !actual
                    .keys()
                    .any(|key| key.as_str().contains("ignored-cycle"))
            );
            assert_eq!(
                actual,
                hashes_with_defaults(&root, &inputs, mode, true).unwrap()
            );
            if let Some(previous) = &previous {
                assert_eq!(&actual, previous);
            }
            previous = Some(actual);
        }
    }
}

#[test]
fn default_link_walk_preserves_gitignore_selection() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("shared/ignored-dir").create_dir_all().unwrap();
    for path in [
        "keep.txt",
        "skip.txt",
        "nested.skip",
        "ignored-dir/file.txt",
    ] {
        at(&format!("shared/{path}"))
            .create_with_contents("data")
            .unwrap();
    }
    at(".gitignore")
        .create_with_contents(
            "packages/a/src/linked/skip.txt\npackages/a/src/linked/ignored-dir/\n",
        )
        .unwrap();
    at("shared/.gitignore")
        .create_with_contents("nested.skip\n")
        .unwrap();
    symlink("../../../shared", at("packages/a/src/linked")).unwrap();
    test_utils::commit_all(&root);
    let before = hashes(&root, &[], 2).unwrap();
    assert!(before.contains_key(&RelativeUnixPathBuf::new("src/linked/keep.txt").unwrap()));
    for excluded in [
        "src/linked/skip.txt",
        "src/linked/nested.skip",
        "src/linked/ignored-dir/file.txt",
    ] {
        assert!(
            !before.contains_key(&RelativeUnixPathBuf::new(excluded).unwrap()),
            "{excluded}"
        );
    }
    at("shared/skip.txt")
        .create_with_contents("changed")
        .unwrap();
    at("shared/nested.skip")
        .create_with_contents("changed")
        .unwrap();
    assert_eq!(before, hashes(&root, &[], 2).unwrap());
}

#[test]
fn git_fallback_preserves_default_link_ignore_rules() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("shared").create_dir_all().unwrap();
    at("shared/.gitignore")
        .create_with_contents("ignored.txt\n")
        .unwrap();
    at("shared/ignored.txt")
        .create_with_contents("ignored")
        .unwrap();
    at("shared/ok.txt").create_with_contents("before").unwrap();
    at("file.txt").create_with_contents("file").unwrap();
    symlink("../../../shared", at("packages/a/src/link")).unwrap();
    symlink("../../../file.txt", at("packages/a/src/file")).unwrap();
    // No index or HEAD: Git hashing must use its manual fallback.
    assert!(matches!(SCM::new(&root), SCM::Git(_)));
    assert!(!at(".git/index").exists());
    let inputs: &[&str] = &[];
    for defaults in [false, true] {
        at("shared/ok.txt").create_with_contents("before").unwrap();
        let backends = [0, 2];
        let before: Vec<_> = backends
            .iter()
            .map(|&mode| hashes_with_defaults(&root, inputs, mode, defaults).unwrap())
            .collect();
        for result in &before {
            for input in ["src/link", "src/link/ok.txt", "src/file"] {
                assert!(result.contains_key(&RelativeUnixPathBuf::new(input).unwrap()));
            }
            assert!(
                !result.contains_key(&RelativeUnixPathBuf::new("src/link/ignored.txt").unwrap())
            );
        }
        assert_eq!(before[0], before[1]);
        at("shared/ignored.txt")
            .create_with_contents("ignored edit")
            .unwrap();
        for (&mode, previous) in backends.iter().zip(&before) {
            assert_eq!(
                &hashes_with_defaults(&root, inputs, mode, defaults).unwrap(),
                previous,
                "backend {mode}, defaults={defaults}"
            );
        }
        at("shared/ok.txt").create_with_contents("after").unwrap();
        for (&mode, previous) in backends.iter().zip(&before) {
            let actual = hashes_with_defaults(&root, inputs, mode, defaults).unwrap();
            assert_ne!(&actual, previous, "backend {mode}, defaults={defaults}");
            assert_eq!(
                actual,
                hashes_with_defaults(&root, inputs, mode, defaults).unwrap()
            );
        }
    }
    assert!(!at(".git/index").exists());
}

#[test]
fn default_exclusions_preserve_selection_and_symlink_contents() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("shared/dir").create_dir_all().unwrap();
    for path in [
        "packages/a/tracked.txt",
        "shared/file.txt",
        "shared/dir/child.txt",
    ] {
        at(path).create_with_contents("before").unwrap();
    }
    symlink("../../../shared/file.txt", at("packages/a/src/file.txt")).unwrap();
    symlink("../../../shared/dir", at("packages/a/src/dir")).unwrap();
    symlink(".", at("packages/a/src/ignored-cycle")).unwrap();
    test_utils::commit_all(&root);
    at("packages/a/.gitignore")
        .create_with_contents("ignored.txt\ntracked.txt\n")
        .unwrap();
    at("packages/a/ignored.txt")
        .create_with_contents("ignored")
        .unwrap();
    let inputs = ["!src/ignored-cycle/**"];
    let before: Vec<_> = (0..3)
        .map(|mode| {
            let actual = hashes_with_defaults(&root, &inputs, mode, true).unwrap();
            assert!(
                !actual.contains_key(&RelativeUnixPathBuf::new("ignored.txt").unwrap()),
                "backend {mode}"
            );
            if mode != 2 {
                assert!(
                    actual.contains_key(&RelativeUnixPathBuf::new("tracked.txt").unwrap()),
                    "tracked defaults, backend {mode}"
                );
            }
            assert!(actual.contains_key(&RelativeUnixPathBuf::new("src/file.txt").unwrap()));
            assert!(actual.contains_key(&RelativeUnixPathBuf::new("src/dir/child.txt").unwrap()));
            assert_eq!(
                actual,
                hashes_with_defaults(&root, &inputs, mode, true).unwrap()
            );
            assert!(
                hashes_with_defaults(&root, &[] as &[&str], mode, true).is_err(),
                "included cycle, backend {mode}"
            );
            actual
        })
        .collect();
    for path in ["shared/file.txt", "shared/dir/child.txt"] {
        at(path).create_with_contents("after").unwrap();
        for (mode, old) in before.iter().enumerate() {
            let actual = hashes_with_defaults(&root, &inputs, mode, true).unwrap();
            let input = if path == "shared/file.txt" {
                "src/file.txt"
            } else {
                "src/dir/child.txt"
            };
            let input = RelativeUnixPathBuf::new(input).unwrap();
            assert_ne!(
                actual.get(&input),
                old.get(&input),
                "{path}, backend {mode}"
            );
            assert_eq!(
                actual,
                hashes_with_defaults(&root, &inputs, mode, true).unwrap()
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn tracked_ignored_directory_links_preserve_literal_children() {
    for name in ["dir", "dir[1]", "dir\nname"] {
        for mode in 0..2 {
            for with_exclusions in [false, true] {
                let (_tmp, root) = test_utils::tmp_dir();
                test_utils::init_repo(&root);
                let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
                at("packages/a/src").create_dir_all().unwrap();
                let children = [
                    "plain.txt",
                    "line\nbreak.txt",
                    "nested\nfolder/child.txt",
                    "nested[1]/child{one,two}.txt",
                ];
                for target in ["one", "two"] {
                    for child in children.into_iter().chain(["skip.txt"]) {
                        let path = at(&format!("shared/{target}/{child}"));
                        path.parent().unwrap().create_dir_all().unwrap();
                        path.create_with_contents("V1").unwrap();
                    }
                    symlink("line\nbreak.txt", at(&format!("shared/{target}/file-link"))).unwrap();
                    symlink(
                        "nested\nfolder",
                        at(&format!("shared/{target}/directory-link")),
                    )
                    .unwrap();
                    if with_exclusions {
                        symlink(".", at(&format!("shared/{target}/ignored-cycle"))).unwrap();
                    }
                }
                let logical = format!("src/{name}");
                let link = at(&format!("packages/a/{logical}"));
                symlink("../../../shared/one", &link).unwrap();
                assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
                test_utils::commit_all(&root);
                at(".gitignore")
                    .create_with_contents("packages/a/src/\n")
                    .unwrap();
                test_utils::commit_all(&root);
                let escaped = wax::escape(&logical);
                let inputs: Vec<String> = if with_exclusions {
                    vec![
                        format!("!{escaped}/ignored-cycle/**"),
                        format!("!{escaped}/skip.txt"),
                    ]
                } else {
                    Vec::new()
                };
                let capture = || {
                    let inputs: Vec<_> = inputs.iter().map(String::as_str).collect();
                    hashes_with_defaults(&root, &inputs, mode, with_exclusions)
                };
                let key =
                    |child: &str| RelativeUnixPathBuf::new(format!("{logical}/{child}")).unwrap();
                let before = capture().unwrap();
                for child in children
                    .into_iter()
                    .chain(["file-link", "directory-link/child.txt"])
                {
                    assert!(
                        before.contains_key(&key(child)),
                        "missing {child:?}, {name:?}, backend {mode}"
                    );
                }
                assert_eq!(before, capture().unwrap());
                for child in children {
                    let path = at(&format!("shared/one/{child}"));
                    path.create_with_contents("V2").unwrap();
                    let edited = capture().unwrap();
                    assert_ne!(before[&key(child)], edited[&key(child)]);
                    assert_eq!(edited, capture().unwrap());
                    path.create_with_contents("V1").unwrap();
                    assert_eq!(before, capture().unwrap());
                }
                if with_exclusions {
                    assert!(!before.contains_key(&key("skip.txt")));
                    at("shared/one/skip.txt")
                        .create_with_contents("excluded edit")
                        .unwrap();
                    assert_eq!(before, capture().unwrap());
                }
                remove_link(&link);
                symlink("../../../shared/two", &link).unwrap();
                let retargeted = capture().unwrap();
                assert!(retargeted.contains_key(&key("line\nbreak.txt")));
                assert_ne!(before, retargeted);
                assert_eq!(retargeted, capture().unwrap());
                at("shared/two/line\nbreak.txt")
                    .create_with_contents("V3")
                    .unwrap();
                let edited = capture().unwrap();
                assert_ne!(
                    retargeted[&key("line\nbreak.txt")],
                    edited[&key("line\nbreak.txt")]
                );
                assert_eq!(edited, capture().unwrap());
                if !with_exclusions {
                    let cycle = at("shared/two/ignored-cycle");
                    symlink(".", &cycle).unwrap();
                    assert!(capture().is_err(), "included cycle must fail");
                    remove_link(&cycle);
                    assert_eq!(edited, capture().unwrap());
                }
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn default_inputs_include_newline_filenames() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("shared").create_dir_all().unwrap();
    for path in [
        "packages/a/src/ordinary\nfile.txt",
        "shared/linked\nfile.txt",
    ] {
        at(path).create_with_contents("before").unwrap();
    }
    symlink("../../../shared", at("packages/a/src/dir")).unwrap();
    assert!(
        std::fs::symlink_metadata(at("packages/a/src/dir"))
            .unwrap()
            .is_symlink()
    );
    test_utils::commit_all(&root);
    for inputs in [vec![], vec!["!src/unused/**"]] {
        let mut before = Vec::new();
        for mode in 0..3 {
            let hashes = hashes_with_defaults(&root, &inputs, mode, true).unwrap();
            for input in ["src/ordinary\nfile.txt", "src/dir/linked\nfile.txt"] {
                assert!(
                    hashes.contains_key(&RelativeUnixPathBuf::new(input).unwrap()),
                    "backend {mode}"
                );
            }
            assert_eq!(
                hashes,
                hashes_with_defaults(&root, &inputs, mode, true).unwrap()
            );
            before.push(hashes);
        }
        for (path, input) in [
            (
                "packages/a/src/ordinary\nfile.txt",
                "src/ordinary\nfile.txt",
            ),
            ("shared/linked\nfile.txt", "src/dir/linked\nfile.txt"),
        ] {
            at(path).create_with_contents("after").unwrap();
            for (mode, previous) in before.iter().enumerate() {
                let actual = hashes_with_defaults(&root, &inputs, mode, true).unwrap();
                let key = RelativeUnixPathBuf::new(input).unwrap();
                assert_ne!(
                    actual.get(&key),
                    previous.get(&key),
                    "{input:?}, backend {mode}"
                );
                assert_eq!(
                    actual,
                    hashes_with_defaults(&root, &inputs, mode, true).unwrap()
                );
            }
        }
        // Raw default enumeration must not broaden the explicit glob's linked
        // child selection. Git's tracked-file matching remains independent.
        for mode in 0..3 {
            assert!(
                !hashes(&root, &["src/**"], mode)
                    .unwrap()
                    .contains_key(&RelativeUnixPathBuf::new("src/dir/linked\nfile.txt").unwrap())
            );
        }
        at("packages/a/src/ordinary\nfile.txt")
            .create_with_contents("before")
            .unwrap();
        at("shared/linked\nfile.txt")
            .create_with_contents("before")
            .unwrap();
    }
}

#[test]
fn intermediate_links_are_inputs() {
    for (directory, absolute) in [(false, false), (true, false), (false, true), (true, true)] {
        let (_tmp, root) = test_utils::tmp_dir();
        let (_outside_tmp, outside) = test_utils::tmp_dir();
        outside
            .join_component("file.txt")
            .create_with_contents("outside")
            .unwrap();
        test_utils::init_repo(&root);
        let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
        at("packages/a/src").create_dir_all().unwrap();
        for name in ["one", "two"] {
            if directory {
                at(name).create_dir_all().unwrap();
                at(&format!("{name}/child.txt"))
                    .create_with_contents("same")
                    .unwrap();
            } else {
                at(name).create_with_contents("same").unwrap();
            }
        }
        let target = |name: &str| {
            if absolute {
                at(name).as_std_path().to_path_buf()
            } else {
                std::path::PathBuf::from(name)
            }
        };
        symlink(target("one"), at("middle")).unwrap();
        symlink("../../../middle", at("packages/a/src/link")).unwrap();
        assert!(
            std::fs::symlink_metadata(at("middle"))
                .unwrap()
                .is_symlink()
        );
        test_utils::commit_all(&root);
        let before: Vec<_> = (0..3)
            .map(|mode| hashes(&root, &["src/**"], mode).unwrap())
            .collect();
        remove_link(at("middle"));
        symlink(target("two"), at("middle")).unwrap();
        for (mode, previous) in before.iter().enumerate() {
            let actual = hashes(&root, &["src/**"], mode).unwrap();
            assert_ne!(&actual, previous, "directory={directory}, backend {mode}");
            assert_eq!(actual, hashes(&root, &["src/**"], mode).unwrap());
        }
        // Returning to the same chain must restore the same input hashes.
        remove_link(at("middle"));
        symlink(target("one"), at("middle")).unwrap();
        for (mode, previous) in before.iter().enumerate() {
            assert_eq!(&hashes(&root, &["src/**"], mode).unwrap(), previous);
        }
        // Intermediates cannot hide forbidden final targets.
        for forbidden in [
            outside.join_component("file.txt"),
            outside,
            root.join_components(&[".git", "config"]),
            root.join_component(".git"),
        ] {
            // Windows link types are fixed at creation. Recreate the outer
            // link when switching between file and directory targets, so the
            // boundary assertion exercises a readable chain on both platforms.
            remove_link(at("packages/a/src/link"));
            remove_link(at("middle"));
            symlink(forbidden.as_std_path(), at("middle")).unwrap();
            symlink("../../../middle", at("packages/a/src/link")).unwrap();
            std::fs::metadata(at("packages/a/src/link")).unwrap();
            std::fs::canonicalize(at("packages/a/src/link")).unwrap();
            for mode in 0..3 {
                let actual = hashes(&root, &["src/**"], mode);
                assert!(
                    actual.is_err(),
                    "directory={directory}, absolute={absolute}, target={forbidden}, backend \
                     {mode}: {actual:?}; link metadata={:?}; resolved={:?}",
                    std::fs::metadata(at("packages/a/src/link")),
                    std::fs::canonicalize(at("packages/a/src/link"))
                );
            }
        }
        remove_link(at("packages/a/src/link"));
        remove_link(at("middle"));
        symlink("middle", at("middle")).unwrap();
        symlink("../../../middle", at("packages/a/src/link")).unwrap();
        for mode in 0..3 {
            assert!(hashes(&root, &["src/**"], mode).is_err());
        }
    }
}

#[test]
fn link_identity_is_checkout_relative() {
    let mut previous = None;
    for _ in 0..2 {
        let (_tmp, root) = test_utils::tmp_dir();
        test_utils::init_repo(&root);
        let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
        at("packages/a/src").create_dir_all().unwrap();
        at("target").create_with_contents("same").unwrap();
        symlink("target", at("middle")).unwrap();
        symlink("../../../middle", at("packages/a/src/link")).unwrap();
        test_utils::commit_all(&root);
        let actual: Vec<_> = (0..3)
            .map(|mode| hashes(&root, &["src/**"], mode).unwrap())
            .collect();
        if let Some(previous) = &previous {
            assert_eq!(&actual, previous);
        }
        previous = Some(actual);
    }
}

#[test]
fn resolution_expands_links_before_parent_components() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    for dir in ["one", "two"] {
        at(&format!("{dir}/child")).create_dir_all().unwrap();
        at(&format!("{dir}/file.txt"))
            .create_with_contents("same")
            .unwrap();
    }
    symlink("one/child", at("pivot")).unwrap();
    symlink("pivot/../file.txt", at("middle")).unwrap();
    symlink("../../../middle", at("packages/a/src/link")).unwrap();
    test_utils::commit_all(&root);
    let before: Vec<_> = (0..3)
        .map(|mode| hashes(&root, &["src/**"], mode).unwrap())
        .collect();
    remove_link(at("pivot"));
    symlink("two/child", at("pivot")).unwrap();
    for (mode, previous) in before.iter().enumerate() {
        let actual = hashes(&root, &["src/**"], mode).unwrap();
        assert_ne!(&actual, previous, "backend {mode}");
        assert_eq!(actual, hashes(&root, &["src/**"], mode).unwrap());
    }
}

// A thread-local counter makes the no-extra-walk contract observable without
// timing assertions or shared counters between parallel tests.
#[test]
fn default_link_fast_path_skips_only_complete_link_free_packages() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("packages/a/src/a.txt")
        .create_with_contents("plain")
        .unwrap();
    at("packages/b").create_dir_all().unwrap();
    at("shared").create_dir_all().unwrap();
    at("shared/target.txt")
        .create_with_contents("linked")
        .unwrap();
    symlink("../../shared/target.txt", at("packages/b/link.txt")).unwrap();
    test_utils::commit_all(&root);
    let scm = SCM::new(&root);
    let package = AnchoredSystemPathBuf::from_raw("packages/a").unwrap();
    let index = scm.build_repo_index_eager().unwrap();
    assert!(index.has_link_free_defaults(at("packages/a").as_std_path()));
    assert!(!index.has_link_free_defaults(at("packages/b").as_std_path()));
    let calls = || crate::package_deps::LINK_SUPPLEMENTS.with(|count| count.get());
    let before = calls();
    let result = scm
        .get_package_file_hashes(&root, &package, &[] as &[&str], false, None, Some(&index))
        .unwrap();
    assert_eq!(
        calls(),
        before,
        "ordinary defaults must not run a link supplement"
    );
    assert_eq!(result, hashes(&root, &[], 2).unwrap());
    assert_eq!(
        result,
        scm.get_package_file_hashes(&root, &package, &[] as &[&str], false, None, Some(&index))
            .unwrap()
    );
    assert_eq!(calls(), before);
    let _ = scm
        .get_package_file_hashes(&root, &package, &["src/**"], false, None, Some(&index))
        .unwrap();
    assert_eq!(calls(), before + 1, "explicit globs retain link discovery");
    let _ = scm
        .get_package_file_hashes(&root, &package, &[] as &[&str], false, None, None)
        .unwrap();
    assert_eq!(
        calls(),
        before + 2,
        "missing index is not evidence of no links"
    );

    let mut tracked = scm.build_tracked_repo_index_eager().unwrap();
    assert!(!tracked.has_link_free_defaults(at("packages/a").as_std_path()));
    scm.populate_repo_index_untracked(
        &mut tracked,
        &[RelativeUnixPathBuf::new("packages/a/src").unwrap()],
    )
    .unwrap();
    assert!(
        !tracked.has_link_free_defaults(at("packages/a").as_std_path()),
        "visiting a package to reach a scoped child does not cover the whole package"
    );
    assert!(tracked.has_link_free_defaults(at("packages/a/src").as_std_path()));

    let mut candidates = scm.build_tracked_repo_index_eager().unwrap();
    candidates.populate_untracked_from_candidates(vec![]);
    assert!(
        !candidates.has_link_free_defaults(at("packages/a").as_std_path()),
        "paths-only candidates cannot prove link types"
    );
}

#[test]
fn default_link_fast_path_rejects_new_and_replaced_links() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("packages/a/src/tracked.txt")
        .create_with_contents("old")
        .unwrap();
    at("shared").create_dir_all().unwrap();
    at("shared/target.txt").create_with_contents("one").unwrap();
    test_utils::commit_all(&root);
    symlink("../../../shared/target.txt", at("packages/a/src/new.txt")).unwrap();
    std::fs::remove_file(at("packages/a/src/tracked.txt")).unwrap();
    symlink(
        "../../../shared/target.txt",
        at("packages/a/src/tracked.txt"),
    )
    .unwrap();
    let scm = SCM::new(&root);
    let index = scm.build_repo_index_eager().unwrap();
    assert!(!index.has_link_free_defaults(at("packages/a").as_std_path()));
    let first = hashes(&root, &[], 1).unwrap();
    assert!(first.contains_key(&RelativeUnixPathBuf::new("src/new.txt").unwrap()));
    assert_eq!(first, hashes(&root, &[], 1).unwrap());
    at("shared/target.txt").create_with_contents("two").unwrap();
    assert_ne!(first, hashes(&root, &[], 1).unwrap());
}

#[test]
fn default_link_fast_path_rejects_ignored_tracked_parent_links() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src/ignored").create_dir_all().unwrap();
    at("packages/a/src/ignored/tracked.txt")
        .create_with_contents("same")
        .unwrap();
    at("shared").create_dir_all().unwrap();
    at("shared/tracked.txt")
        .create_with_contents("same")
        .unwrap();
    test_utils::commit_all(&root);
    at(".gitignore")
        .create_with_contents("packages/a/src/ignored/\n")
        .unwrap();
    std::fs::remove_dir_all(at("packages/a/src/ignored")).unwrap();
    symlink("../../../shared", at("packages/a/src/ignored")).unwrap();
    let scm = SCM::new(&root);
    let index = scm.build_repo_index_eager().unwrap();
    assert!(!index.has_link_free_defaults(at("packages/a").as_std_path()));
    let first = hashes(&root, &[], 1).unwrap();
    assert_eq!(first, hashes(&root, &[], 1).unwrap());
    at("shared/tracked.txt")
        .create_with_contents("edit")
        .unwrap();
    assert_ne!(first, hashes(&root, &[], 1).unwrap());
}

#[test]
fn default_link_fast_path_preserves_ignore_error_policy() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a").create_dir_all().unwrap();
    at("packages/a/file.txt")
        .create_with_contents("ordinary")
        .unwrap();
    test_utils::commit_all(&root);
    at(".ignore").create_with_contents("[z-a]\n").unwrap();
    assert!(
        hashes(&root, &[], 1).is_err(),
        "parent ignore errors remain fatal"
    );
    std::fs::remove_file(at(".ignore")).unwrap();
    at("packages/a/.ignore")
        .create_with_contents("[z-a]\n")
        .unwrap();
    assert!(
        hashes(&root, &[], 1).is_ok(),
        "malformed in-tree rules remain nonfatal"
    );
}

#[test]
fn default_link_fast_path_rejects_pruned_nested_git_directories() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("packages/a/src/a.txt")
        .create_with_contents("plain")
        .unwrap();
    test_utils::commit_all(&root);
    at("packages/a/nested/.git").create_dir_all().unwrap();
    let scm = SCM::new(&root);
    let index = scm.build_repo_index_eager().unwrap();
    assert!(!index.has_link_free_defaults(at("packages/a").as_std_path()));
}

#[test]
fn default_link_fast_path_preserves_global_ignores() {
    const CHILD: &str = "TURBO_TEST_GLOBAL_IGNORE_FILE";
    if let Some(excludes) = std::env::var_os(CHILD) {
        for directory in [false, true] {
            std::fs::write(&excludes, "").unwrap();
            let (_tmp, root) = test_utils::tmp_dir();
            test_utils::init_repo(&root);
            let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
            at("packages/a/src").create_dir_all().unwrap();
            at("shared/target").create_dir_all().unwrap();
            let target = at("shared/target/child.txt");
            target.create_with_contents("before").unwrap();
            symlink(
                if directory {
                    "../../../shared/target"
                } else {
                    "../../../shared/target/child.txt"
                },
                at("packages/a/src/link"),
            )
            .unwrap();
            assert!(
                std::fs::symlink_metadata(at("packages/a/src/link"))
                    .unwrap()
                    .is_symlink()
            );
            test_utils::commit_all(&root);
            std::fs::write(&excludes, "**/link\n**/ignored.txt\n").unwrap();
            at("packages/a/ignored.txt")
                .create_with_contents("untracked and ignored")
                .unwrap();
            let scm = SCM::new(&root);
            let index = scm.build_repo_index_eager().unwrap();
            assert!(
                !index.has_link_free_defaults(at("packages/a").as_std_path()),
                "global ignore rules must not hide tracked links from the scan"
            );
            let before = hashes(&root, &[], 1).unwrap();
            assert_eq!(before, hashes(&root, &[], 0).unwrap());
            assert_eq!(before, hashes(&root, &[], 1).unwrap());
            assert!(!before.contains_key(&RelativeUnixPathBuf::new("ignored.txt").unwrap()));
            let key = RelativeUnixPathBuf::new(if directory {
                "src/link/child.txt"
            } else {
                "src/link"
            })
            .unwrap();
            assert!(before.contains_key(&key));
            target.create_with_contents("after").unwrap();
            let after = hashes(&root, &[], 1).unwrap();
            assert_ne!(before[&key], after[&key]);
            assert_eq!(after, hashes(&root, &[], 0).unwrap());
            assert_eq!(after, hashes(&root, &[], 1).unwrap());
        }
        return;
    }

    // Global-ignore discovery reads environment variables. Isolate them in a
    // child process so parallel tests and the user's Git settings are untouched.
    let temp = tempfile::tempdir().unwrap();
    let excludes = temp.path().join("exclude");
    std::fs::write(&excludes, "").unwrap();
    let config = temp.path().join("gitconfig");
    std::fs::write(
        &config,
        format!(
            "[core]\nexcludesFile = \"{}\"\n",
            excludes.to_str().unwrap().replace('\\', "/")
        ),
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "symlink_inputs_tests::default_link_fast_path_preserves_global_ignores",
            "--nocapture",
        ])
        .env(CHILD, &excludes)
        .env("GIT_CONFIG_GLOBAL", &config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "global-ignore subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn default_link_fast_path_preserves_ignore_overrides() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src/ignored").create_dir_all().unwrap();
    at("packages/a/file.txt")
        .create_with_contents("plain")
        .unwrap();
    at("shared").create_dir_all().unwrap();
    at("shared/target.txt").create_with_contents("one").unwrap();
    at(".gitignore")
        .create_with_contents("packages/a/src/ignored/\n")
        .unwrap();
    test_utils::commit_all(&root);
    symlink(
        "../../../../shared/target.txt",
        at("packages/a/src/ignored/link.txt"),
    )
    .unwrap();
    // .ignore has precedence over .gitignore in default manual link selection.
    at("packages/a/.ignore")
        .create_with_contents("!src/ignored/\n!src/ignored/**\n")
        .unwrap();
    let scm = SCM::new(&root);
    let index = scm.build_repo_index_eager().unwrap();
    assert!(!index.has_link_free_defaults(at("packages/a").as_std_path()));
    let first = hashes(&root, &[], 1).unwrap();
    assert!(first.contains_key(&RelativeUnixPathBuf::new("src/ignored/link.txt").unwrap()));
    assert_eq!(first, hashes(&root, &[], 1).unwrap());
    at("shared/target.txt").create_with_contents("two").unwrap();
    assert_ne!(first, hashes(&root, &[], 1).unwrap());
}

#[test]
fn relocated_links_are_distinct_inputs() {
    // Exercise the first and a later link in the resolution chain, both for
    // a selected file and for files selected through a directory pattern.
    for directory in [false, true] {
        for outer in [false, true] {
            let (_tmp, root) = test_utils::tmp_dir();
            test_utils::init_repo(&root);
            let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
            at("packages/a/src").create_dir_all().unwrap();
            let base = if outer { "shared" } else { "packages/a/src" };
            let suffix = if directory { "c/child.txt" } else { "c.txt" };
            for branch in ["x/b", "a-real/x"] {
                let path = at(&format!("{base}/{branch}/{suffix}"));
                path.parent().unwrap().create_dir_all().unwrap();
                path.create_with_contents("identical contents").unwrap();
            }
            symlink("x", at(&format!("{base}/a"))).unwrap();
            if outer {
                symlink("../../../shared", at("packages/a/src/outer")).unwrap();
            }
            let prefix = if outer { "src/outer" } else { "src" };
            let input = if directory {
                format!("{prefix}/a/b/c/**")
            } else {
                format!("{prefix}/a/b/c.txt")
            };
            test_utils::commit_all(&root);
            let before: Vec<_> = (0..3)
                .map(|mode| hashes(&root, &[&input], mode).unwrap())
                .collect();
            remove_link(at(&format!("{base}/a")));
            std::fs::rename(at(&format!("{base}/a-real")), at(&format!("{base}/a"))).unwrap();
            symlink("x", at(&format!("{base}/a/b"))).unwrap();
            for (mode, previous) in before.iter().enumerate() {
                let actual = hashes(&root, &[&input], mode).unwrap();
                assert_eq!(actual.len(), previous.len());
                assert!(actual.keys().all(|key| previous.contains_key(key)));
                assert_ne!(
                    &actual, previous,
                    "directory={directory}, outer={outer}, backend {mode}"
                );
                assert_eq!(actual, hashes(&root, &[&input], mode).unwrap());
            }
            // Restoring the same topology restores the same identities.
            remove_link(at(&format!("{base}/a/b")));
            std::fs::rename(at(&format!("{base}/a")), at(&format!("{base}/a-real"))).unwrap();
            symlink("x", at(&format!("{base}/a"))).unwrap();
            for (mode, previous) in before.iter().enumerate() {
                assert_eq!(&hashes(&root, &[&input], mode).unwrap(), previous);
            }
        }
    }
}
