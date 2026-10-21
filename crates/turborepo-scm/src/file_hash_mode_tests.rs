#[cfg(unix)]
use turbopath::AbsoluteSystemPath;
use turbopath::{AnchoredSystemPathBuf, RelativeUnixPathBuf};

use crate::{FileHashMode, GitHashes, OidHash, SCM, test_utils};

fn path(value: &str) -> RelativeUnixPathBuf {
    RelativeUnixPathBuf::new(value).unwrap()
}

#[test]
fn content_only_does_not_stat_files() {
    let (_tmp, root) = test_utils::tmp_dir();
    let hashes = GitHashes::from([(
        path("missing"),
        OidHash::from_hex_str("0123456789abcdef0123456789abcdef01234567"),
    )]);
    assert_eq!(
        FileHashMode::ContentOnly
            .apply(&root, hashes.clone(), false)
            .unwrap(),
        hashes
    );
    assert!(
        FileHashMode::ExecutableBit
            .apply(&root, hashes.clone(), true)
            .unwrap()
            .is_empty()
    );
    assert!(
        FileHashMode::ExecutableBit
            .apply(&root, hashes, false)
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn metadata_errors_are_not_treated_as_missing() {
    let (_tmp, root) = test_utils::tmp_dir();
    root.join_component("file")
        .create_with_contents("content")
        .unwrap();
    let hashes = GitHashes::from([(
        path("file/child"),
        OidHash::from_hex_str("0123456789abcdef0123456789abcdef01234567"),
    )]);
    assert!(
        FileHashMode::ExecutableBit
            .apply(&root, hashes, true)
            .is_err()
    );
}

#[test]
fn missing_files_remain_optional_only_for_discovery() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let missing = AnchoredSystemPathBuf::from_raw("missing").unwrap();
    for scm in [SCM::new(&root), SCM::Manual(FileHashMode::ContentOnly)] {
        for mode in [FileHashMode::ContentOnly, FileHashMode::ExecutableBit] {
            let scm = scm.clone().with_file_hash_mode(mode);
            assert!(scm.hash_files(&root, [&missing].into_iter()).is_err());
            assert!(
                scm.hash_discovered_files(&root, [&missing].into_iter())
                    .unwrap()
                    .is_empty()
            );
            assert!(
                scm.get_hashes_for_files(&root, &[&missing], true)
                    .unwrap()
                    .is_empty()
            );
            assert!(scm.get_hashes_for_files(&root, &[&missing], false).is_err());
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
enum Backend {
    Git,
    Index,
    SubprocessIndex,
    Manual,
    Fallback,
}

#[cfg(unix)]
fn package_hash(
    root: &AbsoluteSystemPath,
    backend: Backend,
    mode: FileHashMode,
    inputs: &[&str],
    include_default: bool,
) -> OidHash {
    let mut scm = match backend {
        Backend::Manual => SCM::Manual(mode),
        _ => SCM::new(root).with_file_hash_mode(mode),
    };
    let index = match backend {
        Backend::Index => Some(scm.build_repo_index_eager().unwrap()),
        Backend::SubprocessIndex => Some(
            scm.build_repo_index_from_subprocesses(&[path("packages/pkg")])
                .unwrap(),
        ),
        Backend::Fallback => {
            let SCM::Git(git) = &mut scm else {
                panic!("expected Git")
            };
            git.bin = root.join_component("missing-git");
            None
        }
        _ => None,
    };
    let hashes = scm
        .get_package_file_hashes(
            root,
            &AnchoredSystemPathBuf::from_raw("packages/pkg").unwrap(),
            inputs,
            include_default,
            None,
            index.as_ref(),
        )
        .unwrap();
    hashes[&path("script.sh")]
}

#[cfg(unix)]
fn chmod(root: &AbsoluteSystemPath, filename: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        root.join_unix_path(path(filename)),
        std::fs::Permissions::from_mode(mode),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn executable_permissions_across_backends_and_inputs() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let script = root.join_unix_path(path("packages/pkg/script.sh"));
    script.ensure_dir().unwrap();
    script.create_with_contents("echo hello\n").unwrap();
    root.join_unix_path(path("packages/pkg/package.json"))
        .create_with_contents("{}")
        .unwrap();
    chmod(&root, "packages/pkg/script.sh", 0o644);
    test_utils::commit_all(&root);
    let legacy = package_hash(&root, Backend::Git, FileHashMode::ContentOnly, &[], false);
    let plain = package_hash(&root, Backend::Git, FileHashMode::ExecutableBit, &[], false);
    assert_ne!(legacy, plain);

    for filemode in ["true", "false"] {
        test_utils::require_git_cmd(&root, &["config", "core.filemode", filemode]);
        for (mode, committed) in [(0o755, false), (0o755, true), (0o644, false), (0o644, true)] {
            chmod(&root, "packages/pkg/script.sh", mode);
            if committed {
                test_utils::require_git_cmd(
                    &root,
                    &[
                        "update-index",
                        if mode == 0o755 {
                            "--chmod=+x"
                        } else {
                            "--chmod=-x"
                        },
                        "packages/pkg/script.sh",
                    ],
                );
                test_utils::require_git_cmd(
                    &root,
                    &["commit", "--allow-empty", "-m", "permissions"],
                );
            }
            let expected =
                package_hash(&root, Backend::Git, FileHashMode::ExecutableBit, &[], false);
            assert_eq!(expected == plain, mode == 0o644);
            for backend in [
                Backend::Git,
                Backend::Index,
                Backend::SubprocessIndex,
                Backend::Manual,
                Backend::Fallback,
            ] {
                for (inputs, defaults) in [
                    (&[][..], false),
                    (&["script.sh"][..], false),
                    (&["script.sh"][..], true),
                ] {
                    assert_eq!(
                        package_hash(
                            &root,
                            backend,
                            FileHashMode::ExecutableBit,
                            inputs,
                            defaults
                        ),
                        expected,
                        "{backend:?}, {inputs:?}, {defaults}, {filemode}, {mode:o}, {committed}"
                    );
                    assert_eq!(
                        package_hash(&root, backend, FileHashMode::ContentOnly, inputs, defaults),
                        legacy
                    );
                }
            }
        }
    }

    for (mode, executable) in [
        (0o600, false),
        (0o666, false),
        (0o744, true),
        (0o654, true),
        (0o645, true),
        (0o777, true),
    ] {
        chmod(&root, "packages/pkg/script.sh", mode);
        let hash = package_hash(&root, Backend::Git, FileHashMode::ExecutableBit, &[], false);
        assert_eq!(hash == plain, !executable);
    }
}

#[cfg(unix)]
#[test]
fn exact_inputs_hash_executable_permissions_without_package_defaults() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    let package = AnchoredSystemPathBuf::from_raw("packages/pkg").unwrap();
    let script = root.resolve(&package).join_component("script.sh");
    script.ensure_dir().unwrap();
    script.create_with_contents("echo hello\n").unwrap();
    root.resolve(&package)
        .join_component("package.json")
        .create_with_contents("{}")
        .unwrap();
    test_utils::commit_all(&root);

    for backend in [
        Backend::Git,
        Backend::Index,
        Backend::SubprocessIndex,
        Backend::Manual,
        Backend::Fallback,
    ] {
        for mode in [FileHashMode::ContentOnly, FileHashMode::ExecutableBit] {
            let mut scm = match backend {
                Backend::Manual => SCM::Manual(mode),
                _ => SCM::new(&root).with_file_hash_mode(mode),
            };
            let index = match backend {
                Backend::Index => Some(scm.build_repo_index_eager().unwrap()),
                Backend::SubprocessIndex => Some(
                    scm.build_repo_index_from_subprocesses(&[path("packages/pkg")])
                        .unwrap(),
                ),
                Backend::Fallback => {
                    let SCM::Git(git) = &mut scm else {
                        panic!("expected Git")
                    };
                    git.bin = root.join_component("missing-git");
                    None
                }
                _ => None,
            };
            let mut hashes = Vec::new();
            for permissions in [0o644, 0o755, 0o644] {
                chmod(&root, "packages/pkg/script.sh", permissions);
                let selected = scm
                    .get_explicit_input_file_hashes(&root, &package, &["script.sh"], index.as_ref())
                    .unwrap();
                assert_eq!(selected.len(), 1, "{backend:?}, {mode:?}");
                hashes.push(selected[&path("script.sh")]);
            }
            assert_eq!(hashes[0], hashes[2], "{backend:?}, {mode:?}");
            assert_eq!(
                hashes[0] == hashes[1],
                mode == FileHashMode::ContentOnly,
                "{backend:?}, {mode:?}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn symlink_hashes_follow_target_permissions() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    root.join_component("target")
        .create_with_contents("hello")
        .unwrap();
    std::os::unix::fs::symlink("target", root.join_component("link")).unwrap();
    test_utils::commit_all(&root);
    let link = AnchoredSystemPathBuf::from_raw("link").unwrap();
    for mode in [FileHashMode::ContentOnly, FileHashMode::ExecutableBit] {
        let mut states = Vec::new();
        for permissions in [0o644, 0o755] {
            chmod(&root, "target", permissions);
            let mut backends = Vec::new();
            for scm in [SCM::new(&root), SCM::Manual(FileHashMode::ContentOnly)] {
                let scm = scm.with_file_hash_mode(mode);
                let hash = scm.hash_files(&root, [&link].into_iter()).unwrap();
                assert_eq!(
                    hash,
                    scm.hash_discovered_files(&root, [&link].into_iter())
                        .unwrap()
                );
                assert_eq!(
                    hash,
                    scm.get_hashes_for_files(&root, &[&link], false).unwrap()
                );
                backends.push(hash);
            }
            assert_eq!(backends[0], backends[1]);
            states.push(backends.remove(0));
        }
        assert_eq!(states[0] == states[1], mode == FileHashMode::ContentOnly);
    }
}

#[test]
fn platform_default_and_aliases_hash_once() {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    root.join_component("file")
        .create_with_contents("hello")
        .unwrap();
    #[cfg(unix)]
    chmod(&root, "file", 0o644);
    let file = AnchoredSystemPathBuf::from_raw("file").unwrap();
    let scm = SCM::new(&root);
    assert_eq!(scm.file_hash_mode(), FileHashMode::ContentOnly);
    let legacy = scm.hash_files(&root, [&file].into_iter()).unwrap();
    let expected = FileHashMode::ExecutableBit
        .apply(&root, legacy, false)
        .unwrap();
    let scm = scm.with_file_hash_mode(FileHashMode::ExecutableBit);
    assert_eq!(scm.clone().file_hash_mode(), FileHashMode::ExecutableBit);
    assert_eq!(
        scm.hash_files(&root, [&file].into_iter()).unwrap(),
        expected
    );
    assert_eq!(
        scm.hash_existing_of(&root, [&file].into_iter()).unwrap(),
        expected
    );
    assert_eq!(
        scm.get_hashes_for_files(&root, &[&file], false).unwrap(),
        expected
    );
    assert_eq!(
        scm.get_hashes_for_files(&root, &[&file], true).unwrap(),
        expected
    );
    assert_eq!(
        expected[&path("file")],
        "9c23df3300626b4eab36fb84bfe6c8d758fc9fef"
    );
}
