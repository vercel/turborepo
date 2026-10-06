use super::*;

#[cfg(unix)]
fn private_tempdir() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn fixture(target: &str) -> Vec<u8> {
    tar_with_links(&[
        ("pkg/bin/tool", Regular, b"tool", ""),
        ("pkg/bin/npm", Kind::Symlink, b"", target),
    ])
}

#[cfg(unix)]
#[test]
fn realistic_node_tree_preserves_links_resources_modes_and_cleanup() {
    let root = "node-v24.0.0-darwin-arm64";
    let entries = [
        (
            "bin/npm",
            Kind::Symlink,
            b"".as_slice(),
            "../lib/node_modules/npm/bin/npm-cli.js",
        ),
        (
            "bin/npx",
            Kind::Symlink,
            b"",
            "../lib/node_modules/npm/bin/npx-cli.js",
        ),
        (
            "bin/corepack",
            Kind::Symlink,
            b"",
            "../lib/node_modules/corepack/dist/corepack.js",
        ),
        ("bin/node", Regular, b"local node fixture", ""),
        (
            "lib/node_modules/npm/bin/npm-cli.js",
            Regular,
            b"#!/usr/bin/env node\nrequire('../lib/cli.js')",
            "",
        ),
        (
            "lib/node_modules/npm/bin/npx-cli.js",
            Regular,
            b"#!/usr/bin/env node\nrequire('./npm-cli.js')",
            "",
        ),
        (
            "lib/node_modules/npm/lib/cli.js",
            Regular,
            b"npm runtime resource",
            "",
        ),
        (
            "lib/node_modules/npm/package.json",
            Regular,
            b"{\"name\":\"npm\"}",
            "",
        ),
        (
            "lib/node_modules/corepack/dist/corepack.js",
            Regular,
            b"corepack fixture",
            "",
        ),
        ("include/node/node.h", Regular, b"node headers", ""),
        ("share/man/man1/node.1", Regular, b"manual page", ""),
        ("LICENSE", Regular, b"license resource", ""),
    ];
    let names: Vec<_> = entries
        .iter()
        .map(|(p, _, _, _)| format!("{root}/{p}"))
        .collect();
    let owned: Vec<_> = entries
        .iter()
        .zip(&names)
        .map(|((_, kind, data, target), name)| (name.as_str(), *kind, *data, *target))
        .collect();
    let bytes = tar_with_links(&owned);
    for (bytes, format) in [(bytes.clone(), Tar), (gzip(&bytes), TarGz)] {
        let extracted = extract(
            &verified(bytes),
            format,
            Limits::new(64 * 1024, 64, 240, 12).unwrap(),
            Layout {
                root,
                required_files: &["bin/node", "lib/node_modules/npm/bin/npm-cli.js"],
            },
        )
        .unwrap();
        for (path, kind, data, target) in entries {
            let path = extracted.root_path().join(path);
            if kind.is_symlink() {
                assert!(
                    fs::symlink_metadata(&path)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                assert_eq!(fs::read_link(&path).unwrap(), Path::new(target));
                assert!(fs::metadata(&path).unwrap().is_file());
            } else {
                assert_eq!(fs::read(&path).unwrap(), data);
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o7777,
                    0o755
                );
            }
        }
        assert_eq!(
            fs::read(extracted.root_path().join("bin/npm")).unwrap(),
            entries[4].2
        );
        assert_eq!(
            fs::read(extracted.root_path().join("bin/npx")).unwrap(),
            entries[5].2
        );
        let staging = extracted.staging_path().to_owned();
        drop(extracted);
        assert!(!staging.exists());
    }
}

#[cfg(unix)]
#[test]
fn safe_link_chains_and_directory_links_in_both_archive_orders() {
    let mut entries = [
        ("pkg/bin/tool", Regular, b"tool".as_slice(), ""),
        ("pkg/bin/npm", Kind::Symlink, b"", "./tool"),
        ("pkg/bin/npx", Kind::Symlink, b"", "npm"),
        ("pkg/commands", Kind::Symlink, b"", "bin"),
    ];
    for _ in 0..2 {
        let extracted = unpack(tar_with_links(&entries), Tar, limits()).unwrap();
        for path in ["bin/npm", "bin/npx", "commands/tool", "commands/npx"] {
            assert_eq!(fs::read(extracted.root_path().join(path)).unwrap(), b"tool");
        }
        let staging = extracted.staging_path().to_owned();
        drop(extracted);
        assert!(!staging.exists());
        entries.reverse();
    }
}

#[cfg(unix)]
#[test]
fn unsafe_targets_and_original_traversal_are_rejected() {
    for target in [
        "",
        "/pkg/bin/tool",
        "//host/tool",
        "../../bin/tool",
        "../../../tool",
        ".././../pkg/bin/tool",
    ] {
        rejected(fixture(target), Tar, limits(), UnsafeLink);
    }
    for target in [
        r"C:\tool",
        "C:tool",
        r"..\tool",
        "tool:stream",
        "NUL",
        "tool.",
        "tool ",
        "é",
        "a\0b",
        "a\nb",
        "PROGRA~1",
    ] {
        rejected(fixture(target), Tar, limits(), UnsafePath);
    }
    for target in [
        "missing",
        "Tool",
        "tool/../tool",
        "tool/.",
        "tool/",
        "./tool//",
    ] {
        rejected(fixture(target), Tar, limits(), UnsafeLink);
    }
    // Lexical normalization would hide traversal through a directory symlink.
    let bytes = tar_with_links(&[
        ("pkg/bin/tool", Regular, b"tool", ""),
        ("pkg/alias", Kind::Symlink, b"", "bin"),
        ("pkg/bin/npm", Kind::Symlink, b"", "../alias/../bin/tool"),
    ]);
    rejected(bytes, Tar, limits(), UnsafeLink);
}

#[cfg(unix)]
#[test]
fn file_and_directory_cycles_are_rejected() {
    for links in [
        vec![("pkg/a", "a")],
        vec![("pkg/a", "b"), ("pkg/b", "a")],
        vec![("pkg/a", "b"), ("pkg/b", "c"), ("pkg/c", "a")],
        vec![("pkg/bin/npm", "..")],
        vec![("pkg/bin/npm", ".")],
        vec![("pkg/a/to_b", "../b"), ("pkg/b/to_a", "../a")],
    ] {
        let mut entries = vec![("pkg/bin/tool", Regular, b"tool".as_slice(), "")];
        entries.extend(
            links
                .into_iter()
                .map(|(path, target)| (path, Kind::Symlink, b"".as_slice(), target)),
        );
        let bytes = tar_with_links(&entries);
        rejected(bytes.clone(), Tar, limits(), LinkCycle);
        rejected(gzip(&bytes), TarGz, limits(), LinkCycle);
    }
}

#[cfg(unix)]
#[test]
fn links_reserve_names_and_cannot_redirect_writes() {
    let cases: [Vec<TarEntry<'_>>; 5] = [
        vec![
            ("pkg/link", Kind::Symlink, b"", "bin"),
            ("pkg/link/tool", Regular, b"overwrite", ""),
        ],
        vec![
            ("pkg/bin/npm", Kind::Symlink, b"", "tool"),
            ("pkg/bin/npm", Regular, b"overwrite", ""),
        ],
        vec![
            ("pkg/bin/npm", Kind::Symlink, b"", "tool"),
            ("pkg/bin/npm", Kind::Symlink, b"", "tool"),
        ],
        vec![
            ("pkg/bin/NPM", Kind::Symlink, b"", "tool"),
            ("pkg/bin/npm", Kind::Symlink, b"", "tool"),
        ],
        vec![
            ("pkg/link", Kind::Symlink, b"", "bin"),
            ("pkg/link/", Directory, b"", ""),
        ],
    ];
    for mut entries in cases {
        for _ in 0..2 {
            let mut fixture = vec![("pkg/bin/tool", Regular, b"tool".as_slice(), "")];
            fixture.extend(entries.iter().copied());
            rejected(tar_with_links(&fixture), Tar, limits(), PathConflict);
            entries.reverse();
        }
    }
}

#[cfg(unix)]
#[test]
fn link_metadata_limits_and_required_files_are_checked() {
    rejected(
        tar_with_links(&[("pkg/bin/npm", Kind::Symlink, b"payload", "tool")]),
        Tar,
        limits(),
        InvalidArchive,
    );
    rejected(
        fixture("././././././././tool"),
        Tar,
        limits(),
        LimitExceeded,
    );
    let small = Limits::new(32768, 32, 12, 8).unwrap();
    rejected(fixture("./././././tool"), Tar, small, LimitExceeded);
    let small = Limits::new(32768, 3, 240, 8).unwrap();
    rejected(fixture("tool"), Tar, small, LimitExceeded);
    let long = "a".repeat(90);
    let bytes = tar_with_links(&[("pkg/bin/npm", Kind::Symlink, b"", &long)]);
    let small = Limits::new(32768, 32, 95, 8).unwrap();
    rejected(bytes, Tar, small, LimitExceeded); // Resolved path, not just raw target.
    let bytes = tar_with_links(&[
        ("pkg/bin/tool", Kind::Symlink, b"", "npm"),
        ("pkg/bin/npm", Regular, b"tool", ""),
    ]);
    rejected(bytes, Tar, limits(), LayoutMismatch); // Required files remain regular.
    let mut bytes = fixture("tool");
    let mut header = tar::Header::from_byte_slice(&bytes[1024..1536]).clone();
    header.as_old_mut().linkname[6] = b'x'; // Bytes after the NUL must not be erased.
    header.set_cksum();
    bytes[1024..1536].copy_from_slice(header.as_bytes());
    rejected(bytes, Tar, limits(), UnsafePath);
}

#[cfg(unix)]
#[test]
fn validation_failure_creates_no_links_and_cleans_owned_staging() {
    let surrounding = private_tempdir();
    let sentinel = surrounding.path().join("sentinel");
    fs::write(&sentinel, b"unchanged").unwrap();
    fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o600)).unwrap();
    for target in ["missing", "npm", "../../sentinel"] {
        let staging = tempfile::Builder::new()
            .tempdir_in(surrounding.path())
            .unwrap();
        fs::set_permissions(staging.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = staging.path().to_owned();
        let mut tree = Tree {
            staging,
            root: "pkg".into(),
            nodes: BTreeMap::new(),
            limits: limits(),
            bytes: 0,
        };
        let bytes = tar_with_links(&[
            ("pkg/bin/tool", Regular, b"tool", ""),
            ("pkg/bin/npx", Kind::Symlink, b"", "tool"),
            ("pkg/bin/npm", Kind::Symlink, b"", target),
        ]);
        let result = extract_tar(&bytes, &mut tree).and_then(|()| tree.finish_links());
        assert!(result.is_err());
        assert!(fs::symlink_metadata(path.join("pkg/bin/npx")).is_err());
        assert!(fs::symlink_metadata(path.join("pkg/bin/npm")).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
        drop(tree);
        assert!(!path.exists());
    }
}

#[cfg(unix)]
#[test]
fn materialization_is_deferred_and_io_failure_cleans_partial_links() {
    let staging = private_tempdir();
    let path = staging.path().to_owned();
    let mut tree = Tree {
        staging,
        root: "pkg".into(),
        nodes: BTreeMap::new(),
        limits: limits(),
        bytes: 0,
    };
    extract_tar(
        &tar_with_links(&[
            ("pkg/bin/tool", Regular, b"tool", ""),
            ("pkg/bin/a", Kind::Symlink, b"", "tool"),
            ("pkg/bin/b", Kind::Symlink, b"", "tool"),
        ]),
        &mut tree,
    )
    .unwrap();
    assert!(fs::symlink_metadata(path.join("pkg/bin/a")).is_err());
    // Simulate a filesystem failure at creation. This is not an attack model:
    // same-user concurrent mutation is outside the private-staging contract.
    fs::write(path.join("pkg/bin/b"), b"occupied").unwrap();
    fs::set_permissions(path.join("pkg/bin/b"), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(tree.finish_links(), Err(Io(e)) if e.kind() == io::ErrorKind::AlreadyExists));
    assert!(
        fs::symlink_metadata(path.join("pkg/bin/a"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(path.join("pkg/bin/b")).unwrap(), b"occupied");
    drop(tree);
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn bounded_gnu_link_paths_and_full_width_targets() {
    let target = "a".repeat(100);
    let name = format!("pkg/bin/{}", "n".repeat(110));
    let metadata = format!("{name}\0");
    let target_path = format!("pkg/bin/{target}");
    // Use GNU long-name metadata for both the target file and the link path.
    let target_metadata = format!("{target_path}\0");
    let bytes = tar_with_links(&[
        ("pkg/bin/tool", Regular, b"tool", ""),
        (
            "././@LongLink",
            Kind::GNULongName,
            target_metadata.as_bytes(),
            "",
        ),
        ("placeholder", Regular, b"resource", ""),
        ("././@LongLink", Kind::GNULongName, metadata.as_bytes(), ""),
        ("placeholder", Kind::Symlink, b"", &target),
    ]);
    let extracted = unpack(bytes.clone(), Tar, limits()).unwrap();
    assert_eq!(
        fs::read(extracted.staging_path().join(&name)).unwrap(),
        b"resource"
    );
    let small = Limits::new(32768, 32, 100, 8).unwrap();
    rejected(bytes, Tar, small, LimitExceeded);
}

#[cfg(not(unix))]
#[test]
fn symlink_platform_is_explicitly_unsupported() {
    rejected(fixture("tool"), Tar, limits(), UnsupportedSymlinkPlatform);
    assert!(
        UnsupportedSymlinkPlatform
            .to_string()
            .contains("unsupported on this platform")
    );
}
