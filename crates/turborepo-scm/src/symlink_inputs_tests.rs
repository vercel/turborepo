#![cfg(unix)]

use std::os::unix::{fs::symlink, net::UnixListener};

use sha1::{Digest, Sha1};
use test_case::test_case;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf,
};

use crate::{Error, GitHashes, OidHash, SCM, test_utils};

fn path(value: &str) -> RelativeUnixPathBuf {
    RelativeUnixPathBuf::new(value).unwrap()
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn symlinked_package_roots_hash_logical_inputs(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, "shared/package/src/input.txt", "input");
    link(&root, "packages/a", "../shared/package");
    test_utils::commit_all(&root);
    for inputs in [&[][..], &["src/**"][..], &["src/input.txt"][..]] {
        assert_contents(
            &backend.hashes(&root, inputs, false).unwrap(),
            &[("src/input.txt", "input")],
        );
    }
}

fn write(root: &AbsoluteSystemPath, name: &str, contents: &str) {
    let file = root.join_unix_path(path(name));
    file.parent().unwrap().create_dir_all().unwrap();
    file.create_with_contents(contents).unwrap();
}

fn link(root: &AbsoluteSystemPath, name: &str, target: impl AsRef<std::path::Path>) {
    let file = root.join_unix_path(path(name));
    file.parent().unwrap().create_dir_all().unwrap();
    symlink(target, file).unwrap();
}

#[derive(Clone, Copy, Debug)]
enum Backend {
    Git,
    Indexed,
    Manual,
}

impl Backend {
    fn hashes(
        self,
        root: &AbsoluteSystemPath,
        inputs: &[&str],
        include_defaults: bool,
    ) -> Result<GitHashes, Error> {
        let package = AnchoredSystemPathBuf::from_raw("packages/a").unwrap();
        let scm = match self {
            Self::Git | Self::Indexed => SCM::new(root),
            Self::Manual => SCM::Manual,
        };
        if !matches!(self, Self::Manual) {
            assert!(matches!(&scm, SCM::Git(_)), "fixture must exercise Git");
        }
        let index = matches!(self, Self::Indexed).then(|| scm.build_repo_index_eager().unwrap());
        scm.get_package_file_hashes(
            root,
            &package,
            inputs,
            include_defaults,
            None,
            index.as_ref(),
        )
    }
}

fn assert_contents(hashes: &GitHashes, entries: &[(&str, &str)]) {
    // Git blob IDs of the expected bytes, independent of SCM's hashing helpers.
    let expected: GitHashes = entries
        .iter()
        .map(|(name, contents)| {
            let mut hash = Sha1::new();
            hash.update(format!("blob {}\0", contents.len()));
            hash.update(contents.as_bytes());
            (
                path(name),
                OidHash::from_hex_str(&hex::encode(hash.finalize())),
            )
        })
        .collect();
    assert_eq!(hashes, &expected);
}

fn fixture(tracked_links: bool) -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
    let (tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, "packages/a/src/control.txt", "unchanged");
    write(&root, "shared/x.txt", "V1");
    write(&root, "shared/dir/y.txt", "D1");
    write(&root, "shared/dir/deep/z.txt", "deep");
    test_utils::commit_all(&root);
    link(&root, "packages/a/src/x.txt", "../../../shared/x.txt");
    link(&root, "packages/a/src/dir", "../../../shared/dir");
    if tracked_links {
        test_utils::commit_all(&root);
    }
    (tmp, root)
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn symlink_input_contents_invalidate_hashes(backend: Backend) {
    // The issue #14815 regression: changing referent bytes must invalidate the
    // logical input even though the symlink text and its Git blob are unchanged.
    for tracked_links in [true, false] {
        let (_tmp, root) = fixture(tracked_links);
        let cases: &[(&[&str], bool, &[&str])] = &[
            (
                &[],
                false,
                &[
                    "src/control.txt",
                    "src/x.txt",
                    "src/dir/y.txt",
                    "src/dir/deep/z.txt",
                ],
            ),
            (
                &["src/x.txt"],
                true,
                &[
                    "src/control.txt",
                    "src/x.txt",
                    "src/dir/y.txt",
                    "src/dir/deep/z.txt",
                ],
            ),
            (
                &["src/**"],
                false,
                &[
                    "src/control.txt",
                    "src/x.txt",
                    "src/dir/y.txt",
                    "src/dir/deep/z.txt",
                ],
            ),
            (
                &["src/x.txt", "src/dir"],
                false,
                &["src/x.txt", "src/dir/y.txt", "src/dir/deep/z.txt"],
            ),
            (
                &["src/dir"],
                false,
                &["src/dir/y.txt", "src/dir/deep/z.txt"],
            ),
            (
                &["src/dir/**/*.txt"],
                false,
                &["src/dir/y.txt", "src/dir/deep/z.txt"],
            ),
            (&["src/x.txt"], false, &["src/x.txt"]),
        ];
        let snapshot = |file_contents, dir_contents| {
            cases
                .iter()
                .map(|(inputs, defaults, selected)| {
                    let hashes = backend
                        .hashes(&root, inputs, *defaults)
                        .unwrap_or_else(|error| {
                            panic!(
                                "{backend:?}, tracked={tracked_links}, inputs={inputs:?}: {error}"
                            )
                        });
                    let entries = [
                        ("src/control.txt", "unchanged"),
                        ("src/x.txt", file_contents),
                        ("src/dir/y.txt", dir_contents),
                        ("src/dir/deep/z.txt", "deep"),
                    ];
                    let expected: Vec<_> = entries
                        .into_iter()
                        .filter(|(name, _)| selected.contains(name))
                        .collect();
                    assert_contents(&hashes, &expected);
                    hashes
                })
                .collect::<Vec<_>>()
        };
        let before = snapshot("V1", "D1");
        write(&root, "shared/x.txt", "V2");
        let file_dirty = snapshot("V2", "D1");
        for (old, new) in before.iter().zip(&file_dirty) {
            assert_eq!(old == new, !old.contains_key(&path("src/x.txt")));
        }
        // Commit only referents: the untracked-link cases must stay untracked.
        test_utils::require_git_cmd(&root, &["add", "shared"]);
        test_utils::require_git_cmd(&root, &["commit", "-m", "change file referent"]);
        assert_eq!(file_dirty, snapshot("V2", "D1"));

        write(&root, "shared/dir/y.txt", "D2");
        let dir_dirty = snapshot("V2", "D2");
        for (old, new) in file_dirty.iter().zip(&dir_dirty) {
            assert_eq!(old == new, !old.contains_key(&path("src/dir/y.txt")));
        }
        test_utils::require_git_cmd(&root, &["add", "shared"]);
        test_utils::require_git_cmd(&root, &["commit", "-m", "change directory referent"]);
        assert_eq!(dir_dirty, snapshot("V2", "D2"));
    }
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn symlink_directory_additions_and_deletions(backend: Backend) {
    for tracked_links in [true, false] {
        let (_tmp, root) = fixture(tracked_links);
        let check = |added: bool, deleted: bool| {
            for inputs in [&[][..], &["src/dir"][..]] {
                let mut expected = vec![("src/dir/deep/z.txt", "deep")];
                if !deleted {
                    expected.push(("src/dir/y.txt", "D1"));
                }
                if added {
                    expected.push(("src/dir/new/n.txt", "new"));
                }
                if inputs.is_empty() {
                    expected.extend([("src/control.txt", "unchanged"), ("src/x.txt", "V1")]);
                }
                assert_contents(&backend.hashes(&root, inputs, false).unwrap(), &expected);
            }
        };
        check(false, false);
        write(&root, "shared/dir/new/n.txt", "new");
        check(true, false);
        test_utils::require_git_cmd(&root, &["add", "shared"]);
        test_utils::require_git_cmd(&root, &["commit", "-m", "add referent child"]);
        check(true, false);
        std::fs::remove_file(root.join_unix_path(path("shared/dir/y.txt"))).unwrap();
        check(true, true);
        test_utils::require_git_cmd(&root, &["add", "shared"]);
        test_utils::require_git_cmd(&root, &["commit", "-m", "delete referent child"]);
        check(true, true);
    }
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn symlink_aliases_nested_links_and_logical_exclusions(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, "shared/data/plain.txt", "plain");
    write(&root, "shared/value.txt", "nested file");
    write(&root, "shared/nested/deep.txt", "nested directory");
    link(&root, "shared/data/file.txt", "../value.txt");
    link(&root, "shared/data/dir", "../nested");
    link(&root, "packages/a/src/first", "../../../shared/data");
    link(
        &root,
        "packages/a/src/second",
        root.join_unix_path(path("shared/data")),
    );
    test_utils::commit_all(&root);
    let inputs = ["src/**", "!src/first/dir/**", "!src/second/file.txt"];
    let check = |contents| {
        assert_contents(
            &backend.hashes(&root, &inputs, false).unwrap(),
            &[
                ("src/first/plain.txt", "plain"),
                ("src/first/file.txt", contents),
                ("src/second/plain.txt", "plain"),
                ("src/second/dir/deep.txt", "nested directory"),
            ],
        );
    };
    check("nested file");
    write(&root, "shared/value.txt", "changed nested file");
    check("changed nested file");
    test_utils::commit_all(&root);
    check("changed nested file");
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn excluded_or_unrelated_unsafe_symlinks_are_not_followed(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    let (_outside_tmp, outside) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, "packages/a/src/safe.txt", "safe");
    write(&root, "shared/data/keep.txt", "kept");
    write(&outside, "secret.txt", "outside");
    link(&root, "packages/a/src/outside", &outside);
    link(
        &root,
        "packages/a/src/metadata",
        root.join_unix_path(path(".git")),
    );
    link(&root, "packages/a/src/cycle", "cycle");
    link(&root, "packages/a/src/dir", "../../../shared/data");
    link(&root, "shared/data/unsafe", &outside);
    test_utils::commit_all(&root);
    assert_contents(
        &backend.hashes(&root, &["src/safe.txt"], false).unwrap(),
        &[("src/safe.txt", "safe")],
    );
    // Exclusions are evaluated on logical paths, before resolving the target,
    // including unsafe descendants reached through an otherwise selected link.
    for include_defaults in [true, false] {
        let inputs = [
            "src/**",
            "!src/outside",
            "!src/metadata",
            "!src/cycle",
            "!src/dir/unsafe",
        ];
        assert_contents(
            &backend.hashes(&root, &inputs, include_defaults).unwrap(),
            &[("src/safe.txt", "safe"), ("src/dir/keep.txt", "kept")],
        );
    }
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn selected_unsafe_symlink_inputs_error(backend: Backend) {
    for kind in [
        "outside file",
        "outside directory",
        "metadata file",
        "metadata directory",
        "chain",
        "link cycle",
        "directory cycle",
    ] {
        let (_tmp, root) = test_utils::tmp_dir();
        let (_outside_tmp, outside) = test_utils::tmp_dir();
        test_utils::init_repo(&root);
        write(&root, "packages/a/src/safe.txt", "safe");
        write(&root, "shared/data/keep.txt", "kept");
        write(&outside, "secret.txt", "outside");
        test_utils::commit_all(&root);
        assert_contents(
            &backend.hashes(&root, &["src/safe.txt"], false).unwrap(),
            &[("src/safe.txt", "safe")],
        );
        let target = match kind {
            "outside file" => outside.join_unix_path(path("secret.txt")),
            "outside directory" => outside.clone(),
            "metadata file" => root.join_unix_path(path(".git/config")),
            "metadata directory" => root.join_unix_path(path(".git")),
            "chain" => {
                link(
                    &root,
                    "shared/hop",
                    outside.join_unix_path(path("secret.txt")),
                );
                root.join_unix_path(path("shared/hop"))
            }
            "link cycle" => {
                link(&root, "shared/one", "two");
                link(&root, "shared/two", "one");
                root.join_unix_path(path("shared/one"))
            }
            "directory cycle" => {
                link(&root, "shared/data/back", ".");
                root.join_unix_path(path("shared/data"))
            }
            _ => unreachable!(),
        };
        link(&root, "packages/a/src/unsafe", target);
        test_utils::commit_all(&root);
        for (inputs, defaults) in [
            (&[][..], false),
            (&["src/unsafe"][..], false),
            (&["src/**"][..], true),
        ] {
            assert!(
                backend.hashes(&root, inputs, defaults).is_err(),
                "{backend:?} must reject {kind}, inputs={inputs:?}"
            );
        }
    }
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn symlink_inputs_cannot_escape_turbo_root_within_git_repo(backend: Backend) {
    let (_tmp, repo_root) = test_utils::tmp_dir();
    test_utils::init_repo(&repo_root);
    write(&repo_root, "shared.txt", "outside task root but inside Git");
    write(&repo_root, "nested/packages/a/src/safe.txt", "safe");
    test_utils::commit_all(&repo_root);
    let root = repo_root.join_unix_path(path("nested"));
    assert_contents(
        &backend.hashes(&root, &["src/safe.txt"], false).unwrap(),
        &[("src/safe.txt", "safe")],
    );
    link(&root, "packages/a/src/unsafe", "../../../../shared.txt");
    test_utils::commit_all(&repo_root);
    assert!(backend.hashes(&root, &["src/unsafe"], false).is_err());
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn missing_and_special_symlink_referents_are_skipped_stably(backend: Backend) {
    for tracked_links in [true, false] {
        let (_tmp, root) = test_utils::tmp_dir();
        test_utils::init_repo(&root);
        write(&root, "packages/a/src/safe.txt", "safe");
        root.join_unix_path(path("shared"))
            .create_dir_all()
            .unwrap();
        test_utils::commit_all(&root);
        let _socket = UnixListener::bind(root.join_unix_path(path("shared/socket"))).unwrap();
        link(&root, "packages/a/src/missing", "../../../shared/missing");
        link(&root, "packages/a/src/socket", "../../../shared/socket");
        if tracked_links {
            test_utils::require_git_cmd(&root, &["add", "packages/a"]);
            test_utils::require_git_cmd(
                &root,
                &["commit", "-m", "track missing and special links"],
            );
        }
        for inputs in [
            &[][..],
            &["src/**"][..],
            &["src/safe.txt", "src/missing", "src/socket"][..],
        ] {
            for _ in 0..2 {
                assert_contents(
                    &backend.hashes(&root, inputs, false).unwrap(),
                    &[("src/safe.txt", "safe")],
                );
            }
        }
    }
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn narrow_inputs_through_directory_links_prune_siblings(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    let (_outside_tmp, outside) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, "shared/data/keep.txt", "keep");
    write(&root, "shared/data/deep/keep.txt", "deep");
    write(&outside, "unused.txt", "unused");
    link(&root, "shared/data/cycle", "cycle");
    link(&root, "shared/data/outside", &outside);
    link(&root, "shared/data/deep/cycle", "cycle");
    link(&root, "packages/a/alias", "../../shared/data");
    test_utils::commit_all(&root);
    for (input, entry) in [
        ("alias/keep.txt", ("alias/keep.txt", "keep")),
        ("alias/deep/*.txt", ("alias/deep/keep.txt", "deep")),
        ("alias/d*/keep.txt", ("alias/deep/keep.txt", "deep")),
    ] {
        assert_contents(&backend.hashes(&root, &[input], false).unwrap(), &[entry]);
    }
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn escaped_prefixes_discover_directory_aliases(backend: Backend) {
    for tracked in [false, true] {
        let (_tmp, root) = test_utils::tmp_dir();
        test_utils::init_repo(&root);
        write(&root, "shared/data/keep.txt", "keep");
        write(&root, "packages/a/control.txt", "control");
        test_utils::commit_all(&root);
        for parent in ["generated:code", "generated[code]"] {
            link(
                &root,
                &format!("packages/a/{parent}/alias"),
                "../../../shared/data",
            );
        }
        if tracked {
            test_utils::commit_all(&root);
        }
        for (prefix, pattern) in [
            ("generated:code", "generated:code"),
            ("generated[code]", r"generated\[code\]"),
        ] {
            for suffix in ["/alias", "/**/*.txt"] {
                let input = format!("{pattern}{suffix}");
                let name = format!("{prefix}/alias/keep.txt");
                assert_contents(
                    &backend.hashes(&root, &[&input], false).unwrap(),
                    &[(&name, "keep")],
                );
            }
        }
    }
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn default_ignores_do_not_become_explicit_through_aliases(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(
        &root,
        ".gitignore",
        "*.log\ncache/\npackages/a/src/ignored.txt\npackages/a/src/ignored-dir/\n",
    );
    write(&root, "packages/a/src/keep.txt", "local");
    write(&root, "shared/data/keep.txt", "keep");
    write(&root, "shared/data/drop.log", "log");
    write(&root, "shared/data/cache/unused.txt", "cache");
    write(&root, "shared/ignored/keep.txt", "explicit");
    write(&root, "shared/ignored/other.txt", "not selected");
    write(&root, "shared/value.txt", "ignored");
    test_utils::commit_all(&root);
    link(&root, "packages/a/src/alias", "../../../shared/data");
    link(
        &root,
        "packages/a/src/ignored.txt",
        "../../../shared/value.txt",
    );
    link(
        &root,
        "packages/a/src/ignored-dir",
        "../../../shared/ignored",
    );
    let defaults = [("src/keep.txt", "local"), ("src/alias/keep.txt", "keep")];
    assert_contents(&backend.hashes(&root, &[], false).unwrap(), &defaults);
    assert_contents(
        &backend.hashes(&root, &["src/k*.txt"], true).unwrap(),
        &defaults,
    );
    let mut expected = defaults.to_vec();
    expected.push(("src/alias/drop.log", "log"));
    assert_contents(
        &backend.hashes(&root, &["src/alias/*.log"], true).unwrap(),
        &expected,
    );
    let mut expected = defaults.to_vec();
    expected.push(("src/ignored-dir/keep.txt", "explicit"));
    assert_contents(
        &backend
            .hashes(&root, &["src/ignored-dir/keep.txt"], true)
            .unwrap(),
        &expected,
    );
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn directory_alias_ignores_use_logical_paths_and_nested_rules(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, ".gitignore", "shared/data/keep.txt\n*.log\n");
    let nested_rules = "private/\n!public.log\n";
    write(&root, "shared/data/.gitignore", nested_rules);
    write(&root, "shared/data/keep.txt", "keep");
    write(&root, "shared/data/public.log", "public");
    write(&root, "shared/data/drop.log", "ignored");
    link(&root, "shared/data/private/cycle", "cycle");
    test_utils::commit_all(&root);
    link(&root, "packages/a/alias", "../../shared/data");
    assert_contents(
        &backend.hashes(&root, &[], false).unwrap(),
        &[
            ("alias/.gitignore", nested_rules),
            ("alias/keep.txt", "keep"),
            ("alias/public.log", "public"),
        ],
    );
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn directory_alias_ignore_files_do_not_read_external_rules(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    let (_outside_tmp, outside) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, "shared/data/keep.txt", "keep");
    write(&outside, "rules", "keep.txt\n");
    link(
        &root,
        "shared/data/.gitignore",
        outside.join_component("rules"),
    );
    test_utils::commit_all(&root);
    link(&root, "packages/a/alias", "../../shared/data");
    // The rules file is not itself a task input. Its contents must not affect
    // discovery of other inputs through this otherwise valid directory alias.
    assert_contents(
        &backend.hashes(&root, &["!alias/.gitignore"], true).unwrap(),
        &[("alias/keep.txt", "keep")],
    );
}

#[test_case(Backend::Git; "git")]
#[test_case(Backend::Indexed; "indexed")]
#[test_case(Backend::Manual; "manual")]
fn public_package_hashing_preserves_direct_authored_git_metadata(backend: Backend) {
    let (_tmp, root) = test_utils::tmp_dir();
    test_utils::init_repo(&root);
    write(&root, "package.json", "{}");
    test_utils::commit_all(&root);
    let config = std::fs::read_to_string(root.join_components(&[".git", "config"])).unwrap();
    let scm = match backend {
        Backend::Git | Backend::Indexed => SCM::new(&root),
        Backend::Manual => SCM::Manual,
    };
    let index = matches!(backend, Backend::Indexed).then(|| scm.build_repo_index_eager().unwrap());
    let hashes = scm
        .get_package_file_hashes(
            &root,
            &AnchoredSystemPathBuf::from_raw("").unwrap(),
            &[".git/config"],
            false,
            None,
            index.as_ref(),
        )
        .unwrap();
    assert_contents(&hashes, &[("package.json", "{}"), (".git/config", &config)]);
}
