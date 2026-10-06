#![allow(clippy::unwrap_used)]

mod links;

use std::{io::Write, mem::discriminant};

use sha2::{Digest, Sha256};
use tar::{
    EntryType as Kind,
    EntryType::{Directory, Regular},
};
use turborepo_download::ExpectedSha256;

use super::{Error::*, Format::*, *};

pub(super) fn fixture_permissions(path: &std::path::Path, mode: u32) {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    #[cfg(not(unix))]
    let _ = (path, mode);
}

fn verified(bytes: Vec<u8>) -> VerifiedArtifact {
    let digest = ExpectedSha256::from_hex(&hex::encode(Sha256::digest(&bytes))).unwrap();
    VerifiedArtifact::verify_bytes(bytes, digest).unwrap()
}
pub(super) fn limits() -> Limits {
    Limits::new(32 * 1024, 32, 240, 8).unwrap()
}
fn layout<'a>(files: &'a [&'a str]) -> Layout<'a> {
    Layout {
        root: "pkg",
        required_files: files,
    }
}
pub(super) fn unpack(
    bytes: Vec<u8>,
    format: Format,
    limits: Limits,
) -> Result<ExtractedArtifact, Error> {
    extract(&verified(bytes), format, limits, layout(&["bin/tool"]))
}
pub(super) fn rejected(bytes: Vec<u8>, format: Format, limits: Limits, expected: Error) {
    match unpack(bytes, format, limits) {
        Ok(_) => panic!("expected {expected}"),
        Err(actual) => assert_eq!(discriminant(&actual), discriminant(&expected), "{actual}"),
    }
}
fn tar(entries: &[(&str, Kind, &[u8])]) -> Vec<u8> {
    tar_with_links(
        &entries
            .iter()
            .map(|&(name, kind, data)| (name, kind, data, "tool"))
            .collect::<Vec<_>>(),
    )
}
type TarEntry<'a> = (&'a str, Kind, &'a [u8], &'a str);
fn tar_with_links(entries: &[TarEntry<'_>]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, kind, data, target) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_mode(0o6755);
        header.set_size(data.len() as u64);
        // Literal fixture paths ensure tar::Builder does not normalize spelling.
        header.as_mut_bytes()[..100].fill(0);
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        if kind.is_symlink() || kind.is_hard_link() {
            let field = &mut header.as_old_mut().linkname;
            field[..target.len()].copy_from_slice(target.as_bytes());
        }
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    builder.into_inner().unwrap()
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
fn valid_tar() -> Vec<u8> {
    tar(&[("pkg/bin/tool", Regular, b"tool")])
}

// Also run with `umask 0700` in a separate process: creation modes alone can
// lose owner access, so extraction must restore private, cleanup-safe modes.
#[test]
fn valid_formats_modes_and_owned_cleanup() {
    let tar = valid_tar();
    for (bytes, format) in [(tar.clone(), Tar), (gzip(&tar), TarGz)] {
        let extracted = unpack(bytes, format, limits()).unwrap();
        assert_eq!(
            fs::read(extracted.root_path().join("bin/tool")).unwrap(),
            b"tool"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode(extracted.staging_path().to_owned()), 0o700);
            assert_eq!(mode(extracted.root_path()), 0o700);
            assert_eq!(mode(extracted.root_path().join("bin")), 0o700);
            assert_eq!(mode(extracted.root_path().join("bin/tool")), 0o755);
        }
        let second = unpack(valid_tar(), Tar, limits()).unwrap();
        assert_ne!(second.staging_path(), extracted.staging_path());
        let staging = extracted.staging_path().to_owned();
        drop(extracted);
        assert!(!staging.exists());
    }
}
#[test]
fn unsafe_portable_names_on_all_hosts() {
    for name in concat!(
        "../tool;/pkg/bin/tool;C:/tool;C:tool;pkg/../tool;pkg/./tool;pkg//tool;",
        r"pkg\tool;\\host\share;\\?\C:\tool;pkg/tool:stream;",
        "pkg/NUL.txt;pkg/con .txt;pkg/COM1;pkg/lPt9.log;pkg/CONIN$;pkg/CONOUT$;",
        "pkg/CLOCK$;pkg/COM².txt;pkg/tool.;pkg/tool ;pkg/PROGRA~1;",
        "pkg/a\0b;pkg/é;pkg/tool?;pkg/a\nb"
    )
    .split(';')
    {
        let bytes = tar(&[(name, Regular, b"")]);
        rejected(bytes.clone(), Tar, limits(), UnsafePath);
        rejected(gzip(&bytes), TarGz, limits(), UnsafePath);
    }
}
#[test]
fn links_and_special_metadata_are_explicitly_unsupported() {
    for kind in [Kind::Link, Kind::GNULongLink] {
        let bytes = tar(&[("pkg/bin/tool", Regular, b"x"), ("pkg/bin/npm", kind, b"")]);
        rejected(bytes, Tar, limits(), UnsupportedLink);
    }
    let diagnostic = UnsupportedLink.to_string();
    assert!(diagnostic.contains("hard links"));
    for kind in [
        Kind::Fifo,
        Kind::Char,
        Kind::Block,
        Kind::GNUSparse,
        Kind::XHeader,
        Kind::XGlobalHeader,
    ] {
        let bytes = tar(&[("pkg/special", kind, b"")]);
        rejected(bytes, Tar, limits(), UnsupportedEntry);
    }
}
#[test]
fn case_collisions_duplicates_and_parent_conflicts() {
    for names in [
        ["pkg/bin/tool", "pkg/bin/tool"],
        ["pkg/Bin/tool", "pkg/bin/other"],
        ["pkg/bin", "pkg/bin/tool"],
    ] {
        let bytes = tar(&names.map(|n| (n, Regular, b"x".as_slice())));
        rejected(bytes, Tar, limits(), PathConflict);
    }
    let explicit = tar(&[
        ("pkg/bin/tool", Regular, b"x"),
        ("pkg/bin/", Directory, b""),
    ]);
    assert!(unpack(explicit, Tar, limits()).is_ok());
    let duplicate = tar(&[("pkg/", Directory, b""), ("pkg/", Directory, b"")]);
    rejected(duplicate, Tar, limits(), PathConflict);
}
#[test]
fn limits_and_expected_layout() {
    for invalid in [
        (0, 1, 1, 1),
        (usize::MAX, 1, 1, 1),
        (1, 0, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 2, 1),
        (1, 1, 1, 0),
    ] {
        assert!(Limits::new(invalid.0, invalid.1, invalid.2, invalid.3).is_err());
    }
    for small in [
        Limits::new(100, 32, 80, 8),
        Limits::new(32768, 2, 240, 8),
        Limits::new(32768, 32, 8, 8),
        Limits::new(32768, 32, 240, 2),
    ] {
        rejected(valid_tar(), Tar, small.unwrap(), LimitExceeded);
    }
    let bytes = valid_tar();
    let exact = Limits::new(bytes.len(), 3, 12, 3).unwrap();
    assert!(unpack(bytes, Tar, exact).is_ok());
    for name in ["other/bin/tool", "pkg/bin/other", "pkg"] {
        rejected(tar(&[(name, Regular, b"")]), Tar, limits(), LayoutMismatch);
    }
    let directory = tar(&[("pkg/bin/tool", Directory, b"")]);
    rejected(directory, Tar, limits(), LayoutMismatch);
    let empty_entries = tar(&[
        ("pkg/a", Regular, b""),
        ("pkg/b", Regular, b""),
        ("pkg/c", Regular, b""),
    ]);
    let small = Limits::new(32768, 2, 240, 8).unwrap();
    rejected(empty_entries, Tar, small, LimitExceeded);
    for layout in [
        Layout {
            root: "pkg/sub",
            required_files: &["bin/tool"],
        },
        layout(&[]),
    ] {
        assert!(matches!(
            extract(&verified(valid_tar()), Tar, limits(), layout),
            Err(LayoutMismatch)
        ));
    }
}
#[test]
fn decompression_limits_checksum_and_truncation() {
    let compressed = gzip(&valid_tar());
    let small = Limits::new(compressed.len() - 1, 32, 12, 8).unwrap();
    rejected(compressed, TarGz, small, LimitExceeded);
    let archive = tar(&[("pkg/bin/tool", Regular, &vec![0; 32769])]);
    rejected(gzip(&archive), TarGz, limits(), LimitExceeded);
    let mut corrupt = valid_tar();
    corrupt[0] ^= 1;
    rejected(corrupt, Tar, limits(), InvalidArchive);
    rejected(valid_tar()[..1024].to_vec(), Tar, limits(), InvalidArchive);
    rejected(valid_tar()[..513].to_vec(), Tar, limits(), InvalidArchive);
    let mut trailing = valid_tar();
    trailing.extend_from_slice(&[1; 512]);
    rejected(trailing, Tar, limits(), InvalidArchive);
    let mut crc = gzip(&valid_tar());
    let index = crc.len() - 8;
    crc[index] ^= 1;
    rejected(crc, TarGz, limits(), InvalidArchive);
    let mut truncated = gzip(&valid_tar());
    truncated.pop();
    rejected(truncated, TarGz, limits(), InvalidArchive);
    let mut multi = gzip(&valid_tar());
    multi.extend_from_slice(&gzip(&vec![0; 32769]));
    rejected(multi, TarGz, limits(), LimitExceeded);
}
#[test]
fn bounded_gnu_long_names_and_trusted_digest() {
    let relative = format!("{}/data", "a".repeat(110));
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o600);
    builder
        .append_data(&mut header, format!("pkg/{relative}"), io::empty())
        .unwrap();
    let long = builder.into_inner().unwrap();
    assert!(extract(&verified(long.clone()), Tar, limits(), layout(&[&relative])).is_ok());
    let small = Limits::new(32768, 32, 16, 8).unwrap();
    rejected(long, Tar, small, LimitExceeded);
    let orphan = tar(&[("././@LongLink", Kind::GNULongName, b"pkg/bin/tool\0")]);
    rejected(orphan, Tar, limits(), InvalidArchive);
    let escape = tar(&[
        ("././@LongLink", Kind::GNULongName, b"pkg/../tool\0"),
        ("pkg/bin/tool", Regular, b""),
    ]);
    rejected(escape, Tar, limits(), UnsafePath);
    // Four raw records but only three nodes: GNU metadata counts too.
    let records = tar(&[
        ("././@LongLink", Kind::GNULongName, b"pkg/bin/tool\0"),
        ("pkg/bin/tool", Regular, b"tool"),
        ("pkg/", Directory, b""),
        ("pkg/bin/", Directory, b""),
    ]);
    let small = Limits::new(32768, 3, 240, 8).unwrap();
    rejected(records, Tar, small, LimitExceeded);
    let digest = ExpectedSha256::from_hex(&"00".repeat(32)).unwrap();
    assert!(matches!(
        VerifiedArtifact::verify_bytes(valid_tar(), digest),
        Err(turborepo_download::Error::DigestMismatch)
    ));
}
#[test]
fn ustar_raw_paths_are_not_silently_normalized() {
    for (name, prefix) in [
        ("pkg/bin/tool", ""),
        ("pkg\\bin\\tool", ""),
        ("tool", "pkg/bin"),
        ("tool", "pkg\\bin"),
    ] {
        let mut header = tar::Header::new_ustar();
        header.set_mode(0o755);
        header.set_size(0);
        let ustar = header.as_ustar_mut().unwrap();
        ustar.name[..name.len()].copy_from_slice(name.as_bytes());
        ustar.prefix[..prefix.len()].copy_from_slice(prefix.as_bytes());
        header.set_cksum();
        let mut builder = tar::Builder::new(Vec::new());
        builder.append(&header, io::empty()).unwrap();
        let bytes = builder.into_inner().unwrap();
        if name.contains('\\') || prefix.contains('\\') {
            rejected(bytes, Tar, limits(), UnsafePath);
        } else {
            assert!(unpack(bytes, Tar, limits()).is_ok());
        }
    }
}
#[test]
fn raw_sizes_are_checked_completely_before_processing() {
    let data = b"pkg/bin/tool\0".as_slice();
    let file = ("pkg/bin/tool", Regular, b"tool".as_slice());
    let name = ("././@LongLink", Kind::GNULongName, data);
    for entries in [vec![file], vec![name, file]] {
        let base = tar(&entries);
        for case in 0..8 {
            let mut bytes = base.clone();
            let mut header = tar::Header::from_byte_slice(&bytes[..512]).clone();
            let len = header.size().unwrap() as u8;
            let mut field = [0; 12];
            field[0] = 0x80;
            field[11] = len;
            match case {
                1..=4 => field[case - 1] |= 1,
                5 => field[0] = 0xff,
                6 => {
                    field = [0; 12];
                    let bad = format!("{len:o}\0x");
                    field[..bad.len()].copy_from_slice(bad.as_bytes());
                }
                7 => {
                    field = header.as_old().size;
                    field[0] = b'+';
                }
                _ => {}
            }
            header.as_old_mut().size = field;
            header.set_cksum();
            bytes[..512].copy_from_slice(header.as_bytes());
            if case == 0 {
                assert!(unpack(bytes, Tar, limits()).is_ok());
            } else {
                rejected(bytes, Tar, limits(), InvalidArchive);
            }
        }
    }
}
#[test]
fn destination_copy_errors_keep_their_io_kind() {
    let result = copy_payload(b"tool".as_slice(), &mut [][..]);
    assert!(matches!(result, Err(Io(e)) if e.kind() == io::ErrorKind::WriteZero));
}
#[test]
fn failure_cleanup_and_no_surrounding_writes() {
    let surrounding = tempfile::tempdir().unwrap();
    fixture_permissions(surrounding.path(), 0o700);
    let sentinel = surrounding.path().join("sentinel");
    fs::write(&sentinel, b"unchanged").unwrap();
    fixture_permissions(&sentinel, 0o600);
    let staging = tempfile::Builder::new()
        .tempdir_in(surrounding.path())
        .unwrap();
    fixture_permissions(staging.path(), 0o700);
    let path = staging.path().to_owned();
    let mut tree = Tree {
        staging,
        root: "pkg".into(),
        nodes: BTreeMap::new(),
        limits: limits(),
        bytes: 0,
    };
    let fixture = tar(&[
        ("pkg/bin/tool", Regular, b"x"),
        ("../sentinel", Regular, b"changed"),
    ]);
    assert!(matches!(extract_tar(&fixture, &mut tree), Err(UnsafePath)));
    assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
    assert!(path.join("pkg/bin/tool").exists());
    drop(tree);
    assert!(!path.exists());
}
