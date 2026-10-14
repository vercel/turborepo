use std::{os::unix::fs::MetadataExt, process::Command};

use super::*;

fn add(path: &std::path::Path, permissions: &str) {
    let entry = format!("group:everyone allow {permissions}");
    let output = Command::new("/bin/chmod")
        .args(["+a", &entry])
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn acl_access_with_unchanged_unix_metadata_fails_before_traffic() {
    for (target, permissions) in [
        ("record", "write"),
        ("directory", "search"),
        ("ancestor", "add_file"),
        ("ancestor", "add_subdirectory"),
        ("ancestor", "delete_child"),
        ("ancestor", "delete"),
        ("ancestor", "writeattr"),
        ("ancestor", "writeextattr"),
        ("ancestor", "writesecurity"),
        ("ancestor", "chown"),
        ("ancestor", "add_file,only_inherit,file_inherit"),
    ] {
        let f = Fixture::new();
        let first = Upstream::new(&f, false, None);
        f.run(&first, FROZEN).unwrap();
        let directory = f.owned.home().join("turborepo/setup-roots");
        let record = fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let path = match target {
            "record" => record.clone(),
            "directory" => directory,
            _ => f.owned.home().to_owned(),
        };
        let before = fs::metadata(&path).unwrap();
        let bytes = fs::read(&record).unwrap();
        let manifest = f.manifest();
        add(&path, permissions);
        let after = fs::metadata(&path).unwrap();
        assert_eq!(
            (before.uid(), before.ino(), before.mode(), before.nlink()),
            (after.uid(), after.ino(), after.mode(), after.nlink())
        );
        let empty = LoopbackServer::new([], |_| {}).unwrap();
        for flags in [
            FROZEN,
            &["--tools-only", "--no-frozen", "--update-lock"][..],
        ] {
            assert!(f.run(&empty, flags).is_err(), "{target}: {permissions}");
            assert_eq!(empty.hits(), 0);
        }
        assert_eq!(fs::read(&record).unwrap(), bytes);
        assert_eq!(f.manifest(), manifest);
        f.no_execution();
    }
}

#[test]
fn inherited_acl_on_new_state_component_fails_before_trust_publication() {
    let f = Fixture::new();
    add(
        f.owned.home(),
        "read,search,file_inherit,directory_inherit,only_inherit",
    );
    let upstream = Upstream::new(&f, false, None);
    assert!(f.run(&upstream, FROZEN).is_err());
    assert_eq!(upstream.hits(), 3);
    let private = f.owned.home().join("turborepo");
    assert_eq!(fs::metadata(&private).unwrap().mode() & 0o777, 0o700);
    assert!(!private.join("setup-roots").exists());
    assert!(f.manifest().is_some()); // Published installation is still untrusted.
    f.no_execution();
}
