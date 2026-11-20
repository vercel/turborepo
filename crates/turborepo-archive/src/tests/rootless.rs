//! An empty layout root accepts archives without a top-level directory.
use super::*;

fn rootless(files: &[&str], bytes: Vec<u8>, format: Format) -> Result<ExtractedArtifact, Error> {
    let layout = Layout {
        root: "",
        required_files: files,
    };
    extract(&verified(bytes), format, limits(), layout)
}

#[test]
fn top_level_entries_extract_below_one_root_with_their_resources() {
    let tar = tar(&[
        ("bin/", Directory, b""),
        ("bin/tool", Regular, b"tool"),
        ("include/google/any.proto", Regular, b"proto"),
        ("readme.txt", Regular, b"readme"),
    ]);
    for (bytes, format) in [(tar.clone(), Tar), (gzip(&tar), TarGz)] {
        let tree = rootless(&["bin/tool"], bytes, format).unwrap();
        let root = tree.root_path();
        assert_eq!(root.parent().unwrap(), tree.staging_path());
        assert_eq!(fs::read(root.join("bin/tool")).unwrap(), b"tool");
        assert_eq!(
            fs::read(root.join("include/google/any.proto")).unwrap(),
            b"proto"
        );
        assert_eq!(fs::read(root.join("readme.txt")).unwrap(), b"readme");
    }
    // Required files are relative to the archive's top level.
    let error = rootless(&["tool"], tar, Tar).err().unwrap();
    assert!(matches!(error, LayoutMismatch), "{error}");
}

#[test]
fn entry_names_are_never_normalized() {
    for name in [
        "./bin/tool",
        "/bin/tool",
        "../bin/tool",
        "bin/../tool",
        "bin//tool",
    ] {
        let bytes = tar(&[(name, Regular, b"tool")]);
        let error = rootless(&["bin/tool"], bytes, Tar).err().unwrap();
        assert!(matches!(error, UnsafePath), "{name}: {error}");
    }
    // The synthetic root is the boundary for links too.
    #[cfg(unix)]
    for target in ["../escape", "../root/bin/tool"] {
        let bytes = tar_with_links(&[
            ("bin/tool", Regular, b"tool", ""),
            ("link", Kind::Symlink, b"", target),
        ]);
        let error = rootless(&["bin/tool"], bytes, Tar).err().unwrap();
        assert!(matches!(error, UnsafeLink), "{target}: {error}");
    }
}

#[test]
fn the_synthetic_root_does_not_spend_the_callers_budgets() {
    // `limits()` allows 32 entries, 240 path bytes, and depth 8.
    let deep = "a/b/c/d/e/f/g/tool";
    let files: Vec<_> = (0..31).map(|i| format!("f{i}")).collect();
    let mut entries: Vec<_> = files
        .iter()
        .map(|f| (f.as_str(), Regular, &b""[..]))
        .collect();
    entries.push(("bin", Regular, b"tool"));
    assert!(rootless(&["bin"], tar(&entries), Tar).is_ok());
    entries.push(("extra", Regular, b""));
    let error = rootless(&["bin"], tar(&entries), Tar).err().unwrap();
    assert!(matches!(error, LimitExceeded), "{error}");
    assert!(rootless(&[deep], tar(&[(deep, Regular, b"tool")]), Tar).is_ok());
    let deeper = "a/b/c/d/e/f/g/h/tool";
    let error = rootless(&[deeper], tar(&[(deeper, Regular, b"tool")]), Tar)
        .err()
        .unwrap();
    assert!(matches!(error, LimitExceeded), "{error}");
}
