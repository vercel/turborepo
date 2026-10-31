use super::*;

fn generations(store: &Store) -> Vec<PathBuf> {
    fs::read_dir(&store.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("generation-")
        })
        .collect()
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn published_files_are_read_only_and_directories_stay_writable() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[tool("node")], populate).unwrap();
    let [generation] = generations(&store).try_into().unwrap();
    assert_eq!(mode(&generation), 0o700);
    let (mut files, mut dirs) = (0, 0);
    walk(&generation.join("tools"), &mut |path| {
        let m = fs::symlink_metadata(path)?;
        if m.is_file() {
            assert_eq!(mode(path) & 0o222, 0, "{path:?} is writable");
            files += 1;
        } else if m.is_dir() {
            assert_ne!(mode(path) & 0o200, 0, "{path:?} is not owner-writable");
            dirs += 1;
        }
        Ok(())
    })
    .unwrap();
    assert!(files > 0 && dirs > 0);
    for dir in ["tools", "bin"] {
        assert_ne!(mode(&generation.join(dir)) & 0o200, 0);
    }
}

#[test]
fn remove_dir_all_removes_published_tools() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[tool("node")], populate).unwrap();
    drop(store);
    fs::remove_dir_all(repo.path().join(".turbo")).unwrap();
}

#[test]
fn failed_staging_leaves_no_stage() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let result = store.reconcile(&[tool("node"), tool("pnpm")], |tool, root| {
        if tool.id == "pnpm" {
            return Err(Error::InvalidInventory);
        }
        populate(tool, root)
    });
    assert!(result.is_err());
    assert!(generations(&store).is_empty(), "no stage left behind");
}

#[test]
fn an_orphan_is_removed_by_cleanup() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let result = store.reconcile_checked(&[tool("node")], populate, || {
        Err(io::Error::other("source changed").into())
    });
    assert!(result.is_err());
    assert_eq!(generations(&store).len(), 1);
    store.remove_inactive(Duration::ZERO).unwrap();
    assert!(generations(&store).is_empty());
}

#[test]
fn writable_generations_from_older_releases_stay_valid_and_removable() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[tool("node")], populate).unwrap();
    let mut inventory: Inventory = serde_json::from_slice(&manifest(&store)).unwrap();
    let old = store.root.join(&inventory.generation);
    walk(&old, &mut |path| {
        if fs::symlink_metadata(path)?.is_file() {
            fs::set_permissions(path, fs::Permissions::from_mode(mode(path) | 0o200))?;
        }
        Ok(())
    })
    .unwrap();
    for installed in &mut inventory.tools {
        let tree = old.join("tools").join(&installed.tool.id);
        installed.tree_sha256 = tree_hash(&tree).unwrap();
    }
    fs::write(
        store.root.join("manifest.json"),
        serde_json::to_vec(&inventory).unwrap(),
    )
    .unwrap();
    assert!(
        Store::inspect_with(repo.path(), Verification::Full)
            .unwrap()
            .is_some()
    );

    store
        .reconcile(&[tool("node"), tool("pnpm")], |tool, root| {
            assert_eq!(tool.id, "pnpm", "the writable node tree is copied");
            populate(tool, root)
        })
        .unwrap();
    assert_ne!(mode(&old.join("tools/node/resource")) & 0o200, 0);
    let current = store.current().unwrap().unwrap();
    let copy = current.tool_tree(&tool("node")).unwrap();
    assert_eq!(mode(&copy.join("resource")), 0o444);
    assert_eq!(mode(&copy.join("bin/tool")), 0o555);
    assert!(
        Store::inspect_with(repo.path(), Verification::Full)
            .unwrap()
            .is_some()
    );
    store.remove_inactive(Duration::ZERO).unwrap();
    assert!(!old.exists());
}

#[test]
fn reusing_a_read_only_tree_keeps_its_recorded_hash() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[tool("node")], populate).unwrap();
    let before: Inventory = serde_json::from_slice(&manifest(&store)).unwrap();
    store
        .reconcile(&[tool("node"), tool("pnpm")], populate)
        .unwrap();
    let after: Inventory = serde_json::from_slice(&manifest(&store)).unwrap();
    assert_eq!(after.tools[0].tree_sha256, before.tools[0].tree_sha256);
}
