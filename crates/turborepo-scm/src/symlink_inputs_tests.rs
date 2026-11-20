#![cfg(unix)]

use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{GitHashes, SCM, test_utils};

fn hashes(
    root: &AbsoluteSystemPath,
    inputs: &[&str],
    backend: usize,
) -> Result<GitHashes, crate::Error> {
    let scm = if backend == 2 {
        SCM::Manual(crate::FileHashMode::ContentOnly)
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
fn linked_tracked_outputs_use_referent_contents_and_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let package = AnchoredSystemPathBuf::from_raw("packages/a").unwrap();
    let dir = root.resolve(&package);
    dir.create_dir_all().unwrap();
    let shared = root.join_component("shared");
    shared.create_dir_all().unwrap();
    let target = shared.join_component("script.sh");
    target.create_with_contents("echo first\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::os::unix::fs::symlink("../../shared/script.sh", dir.join_component("script.sh")).unwrap();
    std::os::unix::fs::symlink("../../shared", dir.join_component("linked-dir")).unwrap();
    test_utils::commit_all(&root);
    dir.join_component("generated.sh")
        .create_with_contents("generated")
        .unwrap();
    shared
        .join_component("generated.sh")
        .create_with_contents("generated referent")
        .unwrap();
    for indexed in [false, true] {
        for mode in [
            crate::FileHashMode::ContentOnly,
            crate::FileHashMode::ExecutableBit,
        ] {
            let scm = SCM::new(&root).with_file_hash_mode(mode);
            let read = || {
                let index = indexed.then(|| scm.build_repo_index_eager().unwrap());
                scm.get_tracked_output_hashes(
                    &root,
                    &package,
                    &["**/*.sh".to_owned()],
                    &[],
                    index.as_ref(),
                )
                .unwrap()
            };
            let before = read();
            assert_eq!(before.len(), 2, "indexed {indexed}, mode {mode:?}");
            assert!(
                before.contains_key(&RelativeUnixPathBuf::new("linked-dir/script.sh").unwrap())
            );
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
            let executable = read();
            assert_eq!(
                before == executable,
                mode == crate::FileHashMode::ContentOnly
            );
            target.create_with_contents("echo second\n").unwrap();
            assert_ne!(executable, read());
            target.create_with_contents("echo first\n").unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
}

#[test]
fn explicit_link_selection_keeps_authored_provenance() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let package = AnchoredSystemPathBuf::from_raw("packages/a").unwrap();
    let dir = root.resolve(&package);
    dir.create_dir_all().unwrap();
    root.join_component("shared.txt")
        .create_with_contents("shared")
        .unwrap();
    dir.join_component("package.json")
        .create_with_contents("{}")
        .unwrap();
    std::os::unix::fs::symlink("../../shared.txt", dir.join_component("selected.txt")).unwrap();
    test_utils::commit_all(&root);
    for backend in 0..3 {
        let scm = if backend == 2 {
            SCM::Manual(crate::FileHashMode::ContentOnly)
        } else {
            SCM::new(&root)
        };
        let index = (backend == 1).then(|| scm.build_repo_index_eager().unwrap());
        let hashes = scm
            .get_explicit_input_file_hashes(&root, &package, &["selected.txt"], index.as_ref())
            .unwrap();
        assert_eq!(hashes.len(), 1, "backend {backend}");
        assert!(hashes.contains_key(&RelativeUnixPathBuf::new("selected.txt").unwrap()));
        assert!(
            scm.get_explicit_input_file_hashes(
                &root,
                &package,
                &["selected.txt", "!selected.txt"],
                index.as_ref()
            )
            .unwrap()
            .is_empty()
        );
    }
}

#[test]
fn linked_input_permission_mode_follows_referent() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let package = AnchoredSystemPathBuf::from_raw("packages/a").unwrap();
    let dir = root.resolve(&package);
    dir.create_dir_all().unwrap();
    let target = root.join_component("script.sh");
    target.create_with_contents("echo hello\n").unwrap();
    std::os::unix::fs::symlink("../../script.sh", dir.join_component("script.sh")).unwrap();
    test_utils::commit_all(&root);
    for backend in 0..3 {
        for mode in [
            crate::FileHashMode::ContentOnly,
            crate::FileHashMode::ExecutableBit,
        ] {
            let scm = if backend == 2 {
                SCM::Manual(mode)
            } else {
                SCM::new(&root).with_file_hash_mode(mode)
            };
            let read = || {
                let index = (backend == 1).then(|| scm.build_repo_index_eager().unwrap());
                scm.get_package_file_hashes(
                    &root,
                    &package,
                    &["script.sh"],
                    false,
                    None,
                    index.as_ref(),
                )
                .unwrap()
            };
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
            let before = read();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                before == read(),
                mode == crate::FileHashMode::ContentOnly,
                "backend {backend}, mode {mode:?}"
            );
        }
    }
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
fn symlink_inputs_hash_external_and_git_metadata_targets_as_link_text() {
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
    let link_key = RelativeUnixPathBuf::new("src/link").unwrap();
    for target in [
        outside.join_component("file.txt"),
        outside.clone(),
        root.join_components(&[".git", "config"]),
        root.join_component(".git"),
    ] {
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let link_text = crate::crlf::hash_bytes_as_blob(target.as_str().as_bytes()).unwrap();
        for committed in [false, true] {
            if committed {
                test_utils::commit_all(&root);
            }
            for mode in 0..3 {
                for inputs in [&["src/link"][..], &[], &["src/**"]] {
                    let result = hashes(&root, inputs, mode).unwrap_or_else(|error| {
                        panic!("backend {mode} failed on {target} for {inputs:?}: {error}")
                    });
                    assert!(
                        result
                            .keys()
                            .all(|key| !key.as_str().starts_with("src/link/")),
                        "backend {mode} hashed contents behind {target} for {inputs:?}"
                    );
                }
                let wildcard = hashes(&root, &["src/**"], mode).unwrap();
                assert_eq!(wildcard.get(&link_key), Some(&link_text), "backend {mode}");
                if mode < 2 {
                    let defaults = hashes(&root, &[], mode).unwrap();
                    assert_eq!(defaults.get(&link_key), Some(&link_text), "backend {mode}");
                }
                let excluded = hashes(&root, &["src/**", "!src/link"], mode).unwrap();
                assert_eq!(excluded.len(), 1);
                assert!(
                    excluded.contains_key(&RelativeUnixPathBuf::new("src/control.txt").unwrap())
                );
            }
        }
        std::fs::remove_file(&link).unwrap();
    }
}

#[test]
fn linked_directories_are_read_once_per_input_walk() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let at = |path: &str| root.join_unix_path(RelativeUnixPathBuf::new(path).unwrap());
    at(".gitignore")
        .create_with_contents("node_modules\n")
        .unwrap();
    for package in ["a", "b", "c", "d"] {
        at(&format!("packages/{package}/node_modules"))
            .create_dir_all()
            .unwrap();
        at(&format!("packages/{package}/index.js"))
            .create_with_contents(package)
            .unwrap();
    }
    for (from, to) in [("a", "b"), ("a", "c"), ("b", "d"), ("c", "d"), ("d", "a")] {
        std::os::unix::fs::symlink(
            format!("../../{to}"),
            at(&format!("packages/{from}/node_modules/{to}")),
        )
        .unwrap();
    }
    test_utils::commit_all(&root);
    let blob = |bytes: &str| crate::crlf::hash_bytes_as_blob(bytes.as_bytes()).unwrap();
    let expected: GitHashes = [
        ("index.js", blob("a")),
        ("node_modules/b/index.js", blob("b")),
        ("node_modules/b/node_modules/d/index.js", blob("d")),
        (
            "node_modules/b/node_modules/d/node_modules/a",
            blob("../../a"),
        ),
        ("node_modules/c/index.js", blob("c")),
        ("node_modules/c/node_modules/d", blob("../../d")),
    ]
    .into_iter()
    .map(|(path, hash)| (RelativeUnixPathBuf::new(path).unwrap(), hash))
    .collect();
    for mode in 0..3 {
        assert_eq!(
            hashes(&root, &["**"], mode).unwrap(),
            expected,
            "backend {mode}"
        );
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
