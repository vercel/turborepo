//! Windows stat stamps: fresh at publication despite hard-linked `.exe`
//! exports, stale after every change a stamped check must see, and never a
//! read of file bytes.

use std::{
    io::{Seek, SeekFrom},
    os::windows::fs::OpenOptionsExt,
    thread,
};

use super::*;

fn selected(store: &Store) -> (PathBuf, Inventory) {
    let inventory = Store::inventory(&store.root).unwrap().unwrap();
    (store.root.join(&inventory.generation), inventory)
}

/// Whether every selected tree still matches its publication stamp.
fn fresh(store: &Store) -> bool {
    let (generation, inventory) = selected(store);
    let stamps = Stamps::read(&generation);
    inventory.tools.iter().all(|installed| {
        let tree = generation.join("tools").join(&installed.tool.id);
        stamps.fresh(installed, &tree).unwrap()
    })
}

/// A same-size write that keeps the file ID, through a still-open handle.
fn overwrite(path: &Path, offset: u64, bytes: &[u8]) -> File {
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(bytes).unwrap();
    file
}

#[test]
fn publication_stamps_trees_after_hard_linked_exports_and_copies() {
    let repo = tempfile::tempdir().unwrap();
    // Temporary directories are on NTFS here, so these tests stamp for real.
    assert!(windows_stat::stampable(repo.path()).unwrap());
    let mut store = Store::open(repo.path()).unwrap();
    let mut desired = [node(), pnpm()];
    store.reconcile(&desired, populate).unwrap();
    let ids: Vec<_> = Stamps::read(&selected(&store).0)
        .tools
        .into_keys()
        .collect();
    assert_eq!(ids, ["node", "pnpm"]);
    assert!(fresh(&store));
    // Reuse copies node's tree (new file IDs) and stamps the copy.
    desired[1].version = "10.1.0".into();
    store
        .reconcile(&desired, |tool, root| {
            assert_eq!(tool.id, "pnpm", "node is reused");
            populate(tool, root)
        })
        .unwrap();
    assert_eq!(Stamps::read(&selected(&store).0).tools.len(), 2);
    assert!(fresh(&store));
}

#[test]
fn every_change_stales_stamps_and_content_changes_are_damage() {
    for change in 0..11 {
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        store.reconcile(&[node(), pnpm()], populate).unwrap();
        let (generation, _) = selected(&store);
        let tree = generation.join("tools/node");
        let resource = tree.join("resource");
        // Past the stamped state's timestamp tick (about 16 ms at most).
        thread::sleep(Duration::from_millis(100));
        let damaged = match change {
            0 => {
                overwrite(&resource, 0, b"99");
                true
            }
            // Through the `bin` hard link, whose directory entry differs.
            1 => {
                overwrite(&generation.join("bin/node.exe"), 64, b"tampered");
                true
            }
            // Setting the last-write time back advances the change time.
            2 => {
                let modified = fs::metadata(&resource).unwrap().modified().unwrap();
                overwrite(&resource, 0, b"99")
                    .set_modified(modified)
                    .unwrap();
                true
            }
            // The same size under a new file ID.
            3 => {
                fs::remove_file(&resource).unwrap();
                fs::write(&resource, b"99.0.0").unwrap();
                true
            }
            4 => {
                fs::write(tree.join("added"), b"").unwrap();
                true
            }
            5 => {
                fs::remove_file(&resource).unwrap();
                true
            }
            // Without stamps, the full hash sees a same-size write.
            6 => {
                fs::remove_file(generation.join(STAMPS)).unwrap();
                overwrite(&resource, 0, b"99");
                true
            }
            // Identical bytes under a new file ID only stale the stamp,
            7 => {
                fs::copy(&resource, tree.join("copy")).unwrap();
                fs::rename(tree.join("copy"), &resource).unwrap();
                false
            }
            // as do new times or attributes alone,
            8 => {
                let file = OpenOptions::new().write(true).open(&resource).unwrap();
                file.set_modified(SystemTime::UNIX_EPOCH).unwrap();
                false
            }
            9 => {
                let mut permissions = fs::metadata(&resource).unwrap().permissions();
                permissions.set_readonly(true);
                fs::set_permissions(&resource, permissions).unwrap();
                false
            }
            // and malformed stamps are only a cache miss.
            _ => {
                fs::write(generation.join(STAMPS), b"{").unwrap();
                false
            }
        };
        assert!(!fresh(&store), "{change}");
        let inspected = Store::inspect(repo.path());
        assert_eq!(
            matches!(inspected, Err(Error::DamagedContents)),
            damaged,
            "{change}: {:?}",
            inspected.map(|_| ())
        );
        assert_eq!(store.current().is_err(), damaged, "{change}");
    }
}

/// Only full checks (`turbo setup --check`) and reuse read bytes under a
/// forged stamp, so a reused copy never blesses the edit.
#[test]
fn forged_stamps_pass_stamped_checks_but_not_full_checks_or_reuse() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [node(), pnpm()];
    store.reconcile(&desired, populate).unwrap();
    let (generation, inventory) = selected(&store);
    fs::write(generation.join("tools/node/resource"), b"tampered").unwrap();
    assert!(matches!(
        Store::inspect(repo.path()),
        Err(Error::DamagedContents)
    ));
    let mut stamps = Stamps::default();
    for installed in &inventory.tools {
        let tree = generation.join("tools").join(&installed.tool.id);
        let stamp = Stamp {
            tree_sha256: installed.tree_sha256.clone(),
            stat_sha256: tree_digest(&tree, TreeDigest::Stat).unwrap(),
        };
        stamps.tools.insert(installed.tool.id.clone(), stamp);
    }
    fs::write(
        generation.join(STAMPS),
        serde_json::to_vec(&stamps).unwrap(),
    )
    .unwrap();
    assert!(Store::inspect(repo.path()).unwrap().is_some());
    assert!(matches!(
        Store::inspect_with(repo.path(), Verification::Full),
        Err(Error::DamagedContents)
    ));

    let before = manifest(&store);
    assert!(matches!(
        store.reconcile(&[node()], |_, _| panic!("node is reused")),
        Err(Error::InvalidInventory)
    ));
    assert_eq!(manifest(&store), before);
    store
        .force_reconcile_checked(&desired, populate, || Ok(()))
        .unwrap();
    assert!(
        Store::inspect_with(repo.path(), Verification::Full)
            .unwrap()
            .is_some()
    );
}

/// Stamped checks open files for attributes only, so an exclusive handle
/// (or an antivirus content scan) never blocks them; full checks read bytes.
#[test]
fn stamped_checks_never_open_file_contents() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[node(), pnpm()], populate).unwrap();
    let resource = selected(&store).0.join("tools/node/resource");
    // A scanner may briefly hold the new file open.
    let _exclusive = (0..100)
        .find_map(|_| {
            let file = OpenOptions::new().read(true).share_mode(0).open(&resource);
            file.ok().or_else(|| {
                thread::sleep(Duration::from_millis(20));
                None
            })
        })
        .expect("exclusive handle");
    assert!(Store::inspect(repo.path()).unwrap().is_some());
    assert!(store.is_current(&[node(), pnpm()]).unwrap());
    assert!(matches!(
        Store::inspect_with(repo.path(), Verification::Full),
        Err(Error::Io(_))
    ));
}

#[test]
fn only_ntfs_and_refs_volumes_stamp() {
    for (name, stamps) in [
        ("NTFS", true),
        ("ReFS", true),
        ("FAT32", false),
        ("exFAT", false),
        ("ntfs", false),
        ("", false),
    ] {
        let name: Vec<_> = name.encode_utf16().collect();
        assert_eq!(windows_stat::stamps(&name), stamps);
    }
}
