use super::*;

fn selected(store: &Store) -> (PathBuf, Inventory) {
    let inventory = Store::inventory(&store.root).unwrap().unwrap();
    (store.root.join(&inventory.generation), inventory)
}

/// Re-stamp every tree's current stats under its recorded content hash: the
/// one change only a full check can see.
fn forge(store: &Store) {
    let (generation, inventory) = selected(store);
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
}

#[test]
fn stamped_checks_trust_matching_stamps_and_full_checks_reread_bytes() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [tool("node"), tool("pnpm")];
    store.reconcile(&desired, populate).unwrap();
    let (generation, _) = selected(&store);
    let ids: Vec<_> = Stamps::read(&generation).tools.into_keys().collect();
    assert_eq!(ids, ["node", "pnpm"]);
    // The manifest keeps its shape, so readers that predate stamps still parse it.
    assert!(
        !String::from_utf8(manifest(&store))
            .unwrap()
            .contains("stat")
    );

    fs::write(generation.join("tools/node/resource"), b"tampered").unwrap();
    assert!(Store::inspect(repo.path()).is_err());
    forge(&store);
    assert!(Store::inspect(repo.path()).unwrap().is_some());
    assert!(store.current().unwrap().is_some());
    assert!(matches!(
        Store::inspect_with(repo.path(), Verification::Full),
        Err(Error::DamagedContents)
    ));

    // Reuse copies the tree and re-hashes it, so the edit is never blessed.
    let before = manifest(&store);
    assert!(matches!(
        store.reconcile(&[tool("node")], |_, _| panic!("node is reused")),
        Err(Error::InvalidInventory)
    ));
    assert_eq!(manifest(&store), before);
    assert_eq!(
        store
            .force_reconcile_checked(&desired, populate, || Ok(()))
            .unwrap(),
        Outcome::Replaced
    );
    assert!(
        Store::inspect_with(repo.path(), Verification::Full)
            .unwrap()
            .is_some()
    );
}

#[test]
fn stat_changes_and_missing_stamps_fall_back_to_full_hashes() {
    for change in 0..7 {
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        store.reconcile(&[tool("node")], populate).unwrap();
        let (generation, _) = selected(&store);
        let tree = generation.join("tools/node");
        let resource = tree.join("resource");
        let damaged = match change {
            0 => {
                fs::write(&resource, b"a longer adjacent resource").unwrap();
                true
            }
            1 => {
                let mode = fs::Permissions::from_mode(0o700);
                fs::set_permissions(tree.join("bin/tool"), mode).unwrap();
                true
            }
            2 => {
                fs::write(tree.join("added"), b"").unwrap();
                true
            }
            3 => {
                fs::remove_file(tree.join(".resource")).unwrap();
                true
            }
            // Without stamps, the full hash sees even a same-size edit.
            4 => {
                fs::remove_file(generation.join(STAMPS)).unwrap();
                fs::write(&resource, b"tampered resource").unwrap();
                true
            }
            // A stale stamp over identical bytes (a new inode) still passes.
            5 => {
                fs::copy(tree.join("resource"), tree.join("copy")).unwrap();
                fs::rename(tree.join("copy"), tree.join("resource")).unwrap();
                false
            }
            // Malformed stamps are only a cache miss.
            _ => {
                fs::write(generation.join(STAMPS), b"{").unwrap();
                false
            }
        };
        assert_eq!(Store::inspect(repo.path()).is_err(), damaged, "{change}");
        assert_eq!(store.current().is_err(), damaged, "{change}");
    }
}
