#![cfg(unix)]

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{GitHashes, SCM, test_utils};

fn hashes(
    root: &AbsoluteSystemPath,
    inputs: &[&str],
    backend: usize,
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
        false,
        None,
        index.as_ref(),
    )
}

#[test]
fn symlink_contents_invalidate_inputs() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at("packages/a/src").create_dir_all().unwrap();
    at("shared/dir").create_dir_all().unwrap();
    at("shared/x.txt").create_with_contents("V1").unwrap();
    at("shared/dir/y.txt").create_with_contents("D1").unwrap();
    at("shared/dir/skip.txt")
        .create_with_contents("ignored")
        .unwrap();
    std::os::unix::fs::symlink("../../../shared/x.txt", at("packages/a/src/x.txt")).unwrap();
    std::os::unix::fs::symlink("../../../shared/dir", at("packages/a/src/dir")).unwrap();
    test_utils::commit_all(&root);
    let inputs = ["src/**", "!src/dir/skip.txt"];
    let before: Vec<_> = (0..3)
        .map(|mode| hashes(&root, &inputs, mode).unwrap())
        .collect();
    for (mode, result) in before.iter().enumerate() {
        let mut names: Vec<_> = result.keys().map(|path| path.as_str()).collect();
        names.sort();
        assert_eq!(names, ["src/dir/y.txt", "src/x.txt"]);
        assert_eq!(result, &before[0]);
        assert!(
            hashes(&root, &[], mode)
                .unwrap()
                .contains_key(&RelativeUnixPathBuf::new("src/dir/y.txt").unwrap())
        );
        assert_eq!(
            hashes(&root, &["src/dir"], mode).unwrap(),
            hashes(&root, &["src/dir/**/*.txt"], mode).unwrap()
        );
    }
    at("shared/x.txt").create_with_contents("V2").unwrap();
    at("shared/dir/y.txt").create_with_contents("D2").unwrap();
    for (mode, old) in before.iter().enumerate() {
        let new = hashes(&root, &inputs, mode).unwrap();
        for key in old.keys() {
            assert_ne!(old[key], new[key]);
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
        std::os::unix::fs::symlink(&target, &link).unwrap();
        for mode in 0..3 {
            assert!(
                hashes(&root, &["src/link"], mode).is_err(),
                "backend {mode} accepted {target}"
            );
        }
        std::fs::remove_file(&link).unwrap();
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
    std::os::unix::fs::symlink("../../../target.txt", &link).unwrap();
    test_utils::require_git_cmd(&root, &["add", "-f", "packages/a/control.txt"]);
    test_utils::commit_all(&root);
    outside
        .join_component("target.txt")
        .create_with_contents("outside")
        .unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(outside.join_component("target.txt"), &link).unwrap();
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
