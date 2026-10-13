#![allow(clippy::unwrap_used)]

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{collections::BTreeMap, fs, io::Write};

use crate::{Error::*, Format::*, tests::*, zip::extract_zip, *};

fn put(bytes: &mut [u8], at: usize, value: u32, width: usize) {
    bytes[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn zip(entries: &[(&str, u32, &[u8])], deflate: bool, descriptor: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for &(name, mode, data) in entries {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        let payload = if deflate {
            encoder.finish().unwrap()
        } else {
            data.to_vec()
        };
        let (offset, crc) = (bytes.len(), crc32fast::hash(data));
        let flags = if descriptor { 8 } else { 0 };
        let method = if deflate { 8 } else { 0 };
        let mut header = [0; 30];
        put(&mut header, 0, 0x04034b50, 4);
        put(&mut header, 4, 20, 2);
        put(&mut header, 6, flags, 2);
        put(&mut header, 8, method, 2);
        if !descriptor {
            put(&mut header, 14, crc, 4);
            put(&mut header, 18, payload.len() as u32, 4);
            put(&mut header, 22, data.len() as u32, 4);
        }
        put(&mut header, 26, name.len() as u32, 2);
        bytes.extend(header);
        bytes.extend(name.as_bytes());
        bytes.extend(&payload);
        if descriptor {
            bytes.extend(0x08074b50u32.to_le_bytes());
            bytes.extend(crc.to_le_bytes());
            bytes.extend((payload.len() as u32).to_le_bytes());
            bytes.extend((data.len() as u32).to_le_bytes());
        }
        let mut header = [0; 46];
        put(&mut header, 0, 0x02014b50, 4);
        put(&mut header, 4, 0x0314, 2);
        put(&mut header, 6, 20, 2);
        put(&mut header, 8, flags, 2);
        put(&mut header, 10, method, 2);
        put(&mut header, 16, crc, 4);
        put(&mut header, 20, payload.len() as u32, 4);
        put(&mut header, 24, data.len() as u32, 4);
        put(&mut header, 28, name.len() as u32, 2);
        put(&mut header, 38, mode << 16, 4);
        put(&mut header, 42, offset as u32, 4);
        central.extend(header);
        central.extend(name.as_bytes());
    }
    let mut footer = [0; 22];
    put(&mut footer, 0, 0x06054b50, 4);
    put(&mut footer, 8, entries.len() as u32, 2);
    put(&mut footer, 10, entries.len() as u32, 2);
    put(&mut footer, 12, central.len() as u32, 4);
    put(&mut footer, 16, bytes.len() as u32, 4);
    bytes.extend(central);
    bytes.extend(footer);
    bytes
}
fn valid(deflate: bool, descriptor: bool) -> Vec<u8> {
    zip(&[("pkg/bin/tool", 0o106755, b"tool")], deflate, descriptor)
}
fn central(bytes: &[u8]) -> usize {
    let footer = bytes.len() - 22;
    u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as usize
}

#[test]
fn stored_deflate_descriptors_modes_and_cleanup() {
    for deflate in [false, true] {
        for descriptor in [false, true] {
            let artifact = unpack(valid(deflate, descriptor), Zip, limits()).unwrap();
            let path = artifact.staging_path().to_owned();
            assert_eq!(
                fs::read(artifact.root_path().join("bin/tool")).unwrap(),
                b"tool"
            );
            #[cfg(unix)]
            {
                let mode = |p| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
                assert_eq!(mode(path.clone()), 0o700);
                assert_eq!(mode(artifact.root_path().join("bin")), 0o700);
                assert_eq!(mode(artifact.root_path().join("bin/tool")), 0o755);
            }
            drop(artifact);
            assert!(!path.exists());
        }
    }
    let entries = [
        ("pkg/bin/tool", 0o100644, b"tool".as_slice()),
        ("pkg/bin/", 0o040755, b"".as_slice()),
    ];
    assert!(unpack(zip(&entries, true, false), Zip, limits()).is_ok());
    let empty = zip(&[("pkg/bin/tool", 0, b"")], true, false);
    assert!(unpack(empty, Zip, limits()).is_ok());
    // This payload's CRC equals the optional descriptor signature.
    let collision = zip(&[("pkg/bin/tool", 0, &[172, 10, 122, 213])], true, true);
    assert!(unpack(collision, Zip, limits()).is_ok());
    // Unsigned data descriptor and EOCD comments are legal too.
    let mut bytes = valid(true, true);
    let at = central(&bytes);
    bytes.drain(at - 16..at - 12);
    let end = bytes.len() - 22;
    put(&mut bytes, end + 16, (at - 4) as u32, 4);
    put(&mut bytes, end + 20, 4, 2);
    bytes.extend(b"note");
    assert!(unpack(bytes, Zip, limits()).is_ok());
}

#[test]
fn portable_paths_links_types_collisions_and_layout() {
    for name in [
        "../sentinel",
        "/pkg/bin/tool",
        "C:tool",
        "C:/tool",
        "pkg/../tool",
        "pkg/./tool",
        "pkg//tool",
        "pkg\\tool",
        "\\\\host\\share",
        "\\\\?\\C:\\tool",
        "pkg/tool:stream",
        "pkg/NUL.txt",
        "pkg/con .txt",
        "pkg/COM1",
        "pkg/lPt9.log",
        "pkg/tool.",
        "pkg/tool ",
        "pkg/PROGRA~1",
        "pkg/a\0b",
        "pkg/é",
        "pkg/tool?",
        "pkg/a\nb",
    ] {
        rejected(
            zip(&[(name, 0, b"")], false, false),
            Zip,
            limits(),
            UnsafePath,
        );
    }
    for mode in [0o120777, 0o010644, 0o020644, 0o060644, 0o140644] {
        let expected = if mode == 0o120777 {
            UnsupportedLink
        } else {
            UnsupportedEntry
        };
        rejected(
            zip(&[("pkg/bin/tool", mode, b"x")], false, false),
            Zip,
            limits(),
            expected,
        );
    }
    for names in [
        ["pkg/bin/tool", "pkg/bin/tool"],
        ["pkg/Bin/tool", "pkg/bin/other"],
        ["pkg/bin", "pkg/bin/tool"],
    ] {
        rejected(
            zip(&names.map(|n| (n, 0, b"x".as_slice())), false, false),
            Zip,
            limits(),
            PathConflict,
        );
    }
    for name in ["other/bin/tool", "pkg/bin/other", "pkg"] {
        rejected(
            zip(&[(name, 0, b"")], false, false),
            Zip,
            limits(),
            LayoutMismatch,
        );
    }
    rejected(
        zip(&[("pkg/bin/tool/", 0o040755, b"")], false, false),
        Zip,
        limits(),
        LayoutMismatch,
    );
}

#[test]
fn zip_links_stay_rejected_with_tar_link_support() {
    let mut entries = [
        ("pkg/bin/tool", 0o100755, b"tool".as_slice()),
        ("pkg/bin/npm", 0o120777, b"../lib/npm-cli.js".as_slice()),
        ("pkg/lib/npm-cli.js", 0o100755, b"npm".as_slice()),
    ];
    for _ in 0..2 {
        for deflate in [false, true] {
            rejected(zip(&entries, deflate, true), Zip, limits(), UnsupportedLink);
        }
        entries.reverse();
    }
    let bytes = valid(false, false);
    let artifact = unpack(bytes, Zip, limits()).unwrap();
    assert!(
        !fs::symlink_metadata(artifact.root_path().join("bin/tool"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn byte_entry_node_path_and_decompression_budgets() {
    for small in [
        Limits::new(100, 32, 80, 8),
        Limits::new(32768, 2, 240, 8),
        Limits::new(32768, 32, 8, 8),
        Limits::new(32768, 32, 240, 2),
    ] {
        rejected(valid(false, false), Zip, small.unwrap(), LimitExceeded);
    }
    let bytes = valid(false, false);
    assert!(
        unpack(
            bytes.clone(),
            Zip,
            Limits::new(bytes.len(), 3, 12, 3).unwrap()
        )
        .is_ok()
    );
    let bomb = zip(&[("pkg/bin/tool", 0, &vec![0; 32769])], true, false);
    rejected(bomb, Zip, limits(), LimitExceeded);
    let sum = zip(
        &[
            ("pkg/bin/tool", 0, &vec![0; 20000]),
            ("pkg/data", 0, &vec![0; 20000]),
        ],
        true,
        false,
    );
    rejected(sum, Zip, limits(), LimitExceeded);
    let mut count = valid(false, false);
    let end = count.len() - 22;
    put(&mut count, end + 8, 33, 2);
    put(&mut count, end + 10, 33, 2);
    rejected(count, Zip, limits(), LimitExceeded);
    // Understated decoded size cannot bypass the bounded decoder.
    let mut lie = valid(true, false);
    let at = central(&lie);
    put(&mut lie, 22, 1, 4);
    put(&mut lie, at + 24, 1, 4);
    rejected(lie, Zip, limits(), InvalidArchive);
}

#[test]
fn malformed_encrypted_unsupported_and_inconsistent_records() {
    let bytes = valid(true, true);
    // Every truncation must fail without panics or partial success.
    for length in 0..bytes.len() {
        assert!(unpack(bytes[..length].to_vec(), Zip, limits()).is_err());
    }
    for (relative, value, width, unsupported) in [
        (8, 1, 2, true),
        (8, 0x40, 2, true),
        (8, 0x2000, 2, true),
        (10, 99, 2, true),
        (6, 45, 2, true),
        (6, 0, 2, true),
        (6, 10, 2, true),
        (36, 2, 2, true),
        (4, 0x0b14, 2, true),
        (34, 1, 2, true),
        (38, 0x400, 4, true),
        (42, 1, 4, false),
        (20, u32::MAX, 4, true),
        (0, 0, 4, false),
        (16, 0, 4, false),
    ] {
        let mut corrupt = bytes.clone();
        let at = central(&corrupt);
        put(&mut corrupt, at + relative, value, width);
        rejected(
            corrupt,
            Zip,
            limits(),
            if unsupported {
                UnsupportedEntry
            } else {
                InvalidArchive
            },
        );
    }
    for at in [0, 4, 6, 8, 10, 14, 18, 22, 26, 28, 30, 42] {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 1;
        assert!(unpack(corrupt, Zip, limits()).is_err(), "local offset {at}");
    }
    let mut appended = bytes.clone();
    appended.push(0);
    rejected(appended, Zip, limits(), InvalidArchive);
    for relative in [4, 6, 8, 12, 16] {
        let mut corrupt = bytes.clone();
        let at = corrupt.len() - 22 + relative;
        corrupt[at] ^= 1;
        assert!(unpack(corrupt, Zip, limits()).is_err());
    }
    // Corrupt DEFLATE bytes and payload CRC independently.
    let mut corrupt = valid(true, false);
    corrupt[42] = 0xff;
    rejected(corrupt, Zip, limits(), InvalidArchive);
    let mut crc = valid(false, false);
    crc[42] ^= 1;
    rejected(crc, Zip, limits(), InvalidArchive);
}

#[test]
fn record_coverage_and_decoder_end_are_exact() {
    let mut bytes = valid(true, false);
    let at = central(&bytes);
    bytes.insert(at, 0);
    let length = (at - 42 + 1) as u32;
    put(&mut bytes, 18, length, 4);
    put(&mut bytes, at + 1 + 20, length, 4);
    let end = bytes.len() - 22;
    put(&mut bytes, end + 16, (at + 1) as u32, 4);
    rejected(bytes, Zip, limits(), InvalidArchive);
    let base = zip(
        &[("pkg/bin/tool", 0, b"x"), ("pkg/data", 0, b"y")],
        false,
        false,
    );
    for case in 0..3 {
        let mut bytes = base.clone();
        let at = central(&bytes);
        let end = bytes.len() - 22;
        match case {
            0 => put(&mut bytes, at + 58 + 42, 0, 4), // Overlap.
            1 => {
                let reversed = [bytes[at + 58..end].to_vec(), bytes[at..at + 58].to_vec()].concat();
                bytes[at..end].copy_from_slice(&reversed);
            }
            _ => {
                // Hide a complete local record by omitting its index.
                bytes.drain(at + 58..end);
                let end = bytes.len() - 22;
                put(&mut bytes, end + 8, 1, 2);
                put(&mut bytes, end + 10, 1, 2);
                put(&mut bytes, end + 12, 58, 4);
            }
        }
        rejected(bytes, Zip, limits(), InvalidArchive);
    }
}

#[test]
fn extras_are_bounded_validated_and_never_link_metadata() {
    assert!(matches!(
        super::extras(&[0x55, 0x54, 1, 0, 0, 0x55, 0x54, 1, 0, 0], false),
        Err(InvalidArchive)
    ));
    for (id, payload, expected) in [
        (0x5455u16, vec![1, 0, 0, 0, 0], None),
        (0x7875, vec![1, 1, 0, 1, 0], None),
        (
            0x000a,
            [vec![0, 0, 0, 0, 1, 0, 24, 0], vec![0; 24]].concat(),
            None,
        ),
        (0x5455, vec![1], Some(InvalidArchive)),
        (0x7875, vec![1, 255], Some(InvalidArchive)),
        (0x000a, vec![0; 32], Some(InvalidArchive)),
        (0x000d, vec![], Some(UnsupportedEntry)),
        (0x756e, vec![], Some(UnsupportedEntry)),
        (0x9901, vec![], Some(UnsupportedEntry)),
        (0x0001, vec![], Some(UnsupportedEntry)),
    ] {
        for local in [false, true] {
            let mut bytes = valid(false, false);
            let at = central(&bytes);
            let extra = [
                id.to_le_bytes().to_vec(),
                (payload.len() as u16).to_le_bytes().to_vec(),
                payload.clone(),
            ]
            .concat();
            if local {
                put(&mut bytes, 28, extra.len() as u32, 2);
                bytes.splice(42..42, extra.iter().copied());
            } else {
                put(&mut bytes, at + 30, extra.len() as u32, 2);
                bytes.splice(at + 58..at + 58, extra.iter().copied());
            }
            let end = bytes.len() - 22;
            put(
                &mut bytes,
                end + 16,
                (at + if local { extra.len() } else { 0 }) as u32,
                4,
            );
            if !local {
                put(&mut bytes, end + 12, (58 + extra.len()) as u32, 4);
            }
            if let Some(ref error) = expected {
                let actual = unpack(bytes, Zip, limits()).err().unwrap();
                assert_eq!(
                    std::mem::discriminant(&actual),
                    std::mem::discriminant(error)
                );
            } else {
                assert!(unpack(bytes, Zip, limits()).is_ok());
            }
        }
    }
}

#[test]
fn all_single_byte_mutations_are_bounded_and_never_change_verified_payload() {
    for deflate in [false, true] {
        let original = valid(deflate, true);
        for at in 0..original.len() {
            for mask in [1, 128, 255] {
                let mut bytes = original.clone();
                bytes[at] ^= mask;
                if let Ok(artifact) = unpack(bytes, Zip, limits()) {
                    assert_eq!(
                        fs::read(artifact.root_path().join("bin/tool")).unwrap(),
                        b"tool"
                    );
                }
            }
        }
    }
}

#[test]
fn failure_cleans_owned_staging_without_surrounding_writes() {
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
    let fixture = zip(
        &[("pkg/bin/tool", 0, b"x"), ("../sentinel", 0, b"changed")],
        true,
        false,
    );
    assert!(matches!(extract_zip(&fixture, &mut tree), Err(UnsafePath)));
    assert!(path.join("pkg/bin/tool").exists());
    assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
    drop(tree);
    assert!(!path.exists());
}
