use super::*;

const RESIDUE: Allowances = Allowances {
    tar_name_residue: true,
    case_sensitive_dir: None,
};
const TERMINFO: Allowances = Allowances {
    tar_name_residue: false,
    case_sensitive_dir: Some("share/terminfo"),
};

fn with(bytes: Vec<u8>, allowances: Allowances) -> Result<ExtractedArtifact, Error> {
    extract_with(
        &verified(bytes),
        Tar,
        limits(),
        layout(&["bin/tool"]),
        allowances,
    )
}
fn fails(result: Result<ExtractedArtifact, Error>, expected: Error) {
    match result {
        Ok(_) => panic!("expected {expected}"),
        Err(actual) => assert_eq!(discriminant(&actual), discriminant(&expected), "{actual}"),
    }
}
/// Like python-build-standalone, which renames `python/install/bin/idle3` to
/// `python/bin/idle3` in place: the old name's tail follows the NUL.
fn renamed(name: &str, tail: &str) -> Vec<u8> {
    let mut bytes = tar(&[("pkg/bin/tool", Regular, b"tool"), (name, Regular, b"")]);
    let mut header = tar::Header::from_byte_slice(&bytes[1024..1536]).clone();
    header.as_old_mut().name[name.len() + 1..][..tail.len()].copy_from_slice(tail.as_bytes());
    header.set_cksum();
    bytes[1024..1536].copy_from_slice(header.as_bytes());
    bytes
}

#[test]
fn tar_name_residue_is_ignored_only_when_allowed() {
    let bytes = renamed("pkg/lib/os.py", "l/os.py");
    fails(with(bytes.clone(), Allowances::default()), UnsafePath);
    let extracted = with(bytes, RESIDUE).unwrap();
    let names: Vec<_> = fs::read_dir(extracted.root_path().join("lib"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["os.py"]);
    // The name before the NUL is still checked.
    fails(with(renamed(r"pkg\os.py", "x"), RESIDUE), UnsafePath);
}

#[cfg(unix)]
#[test]
fn link_name_residue_stays_rejected() {
    let mut bytes = tar_with_links(&[
        ("pkg/bin/tool", Regular, b"tool", ""),
        ("pkg/bin/npm", Kind::Symlink, b"", "tool"),
    ]);
    let mut header = tar::Header::from_byte_slice(&bytes[1024..1536]).clone();
    header.as_old_mut().linkname[5] = b'x';
    header.set_cksum();
    bytes[1024..1536].copy_from_slice(header.as_bytes());
    fails(with(bytes, RESIDUE), UnsafePath);
}

#[test]
fn case_sensitive_dir_must_be_a_portable_relative_path() {
    for dir in [
        "",
        "/share",
        "share/",
        "../share",
        "share/./terminfo",
        r"share\terminfo",
    ] {
        let allowances = Allowances {
            case_sensitive_dir: Some(dir),
            ..Allowances::default()
        };
        fails(with(valid_tar(), allowances), UnsafePath);
    }
}

/// Real terminfo shape: different terminals and links in case-only siblings.
#[cfg(target_os = "linux")]
#[test]
fn linux_keeps_case_distinct_names_below_the_allowed_dir() {
    let terminfo = |extra: &[TarEntry<'static>]| {
        let mut entries: Vec<TarEntry<'static>> = vec![
            ("pkg/bin/tool", Regular, b"tool", ""),
            ("pkg/share/terminfo/E/Eterm", Regular, b"Eterm", ""),
            ("pkg/share/terminfo/e/eterm", Regular, b"eterm", ""),
            (
                "pkg/share/terminfo/E/Eterm-color",
                Kind::Symlink,
                b"",
                "Eterm",
            ),
            ("pkg/share/terminfo/N/NCR", Kind::Symlink, b"", "../n/ncr"),
            ("pkg/share/terminfo/n/ncr", Regular, b"ncr", ""),
        ];
        entries.extend_from_slice(extra);
        tar_with_links(&entries)
    };
    let extracted = with(terminfo(&[]), TERMINFO).unwrap();
    let root = extracted.root_path().join("share/terminfo");
    for (path, bytes) in [
        ("E/Eterm", "Eterm"),
        ("e/eterm", "eterm"),
        ("E/Eterm-color", "Eterm"),
        ("N/NCR", "ncr"),
    ] {
        assert_eq!(fs::read(root.join(path)).unwrap(), bytes.as_bytes());
    }
    fails(with(terminfo(&[]), Allowances::default()), PathConflict);
    // Links resolve to the exact spelling only.
    let alias = (
        "pkg/share/terminfo/E/alias",
        Kind::Symlink,
        b"".as_slice(),
        "eterm",
    );
    fails(with(terminfo(&[alias]), TERMINFO), UnsafeLink);
    // Exact duplicates, other spellings of the dir, and paths outside it.
    for names in [
        ["pkg/share/terminfo/e/x", "pkg/share/terminfo/e/x"],
        ["pkg/share/Terminfo/x", "pkg/share/terminfo/y"],
        ["pkg/share/terminfo", "pkg/share/terminfo/x"],
        ["pkg/lib/X", "pkg/lib/x"],
    ] {
        let mut entries: Vec<(&str, Kind, &[u8])> = vec![("pkg/bin/tool", Regular, b"tool")];
        entries.extend(names.map(|name| (name, Regular, b"x".as_slice())));
        fails(with(tar(&entries), TERMINFO), PathConflict);
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn other_targets_still_reject_case_only_collisions() {
    let bytes = tar(&[
        ("pkg/bin/tool", Regular, b"tool"),
        ("pkg/share/terminfo/E/Eterm", Regular, b"Eterm"),
        ("pkg/share/terminfo/e/eterm", Regular, b"eterm"),
    ]);
    fails(with(bytes, TERMINFO), PathConflict);
}
