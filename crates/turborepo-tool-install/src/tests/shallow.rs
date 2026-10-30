use super::*;

fn selected(store: &Store) -> (PathBuf, Inventory) {
    let inventory = Store::inventory(&store.root).unwrap().unwrap();
    (store.root.join(&inventory.generation), inventory)
}

fn shallow(repo: &Path) -> Result<Option<Current>, Error> {
    Store::inspect_with(repo, Verification::Shallow)
}

#[test]
fn shallow_checks_ignore_tree_bytes_that_full_checks_reject() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store
        .reconcile(&[tool("node"), tool("pnpm")], populate)
        .unwrap();
    let (generation, _) = selected(&store);
    let resource = generation.join("tools/node/resource");
    fs::remove_file(&resource).unwrap();
    fs::write(&resource, b"tampered").unwrap();

    let current = shallow(repo.path()).unwrap().unwrap();
    assert_eq!(current.tools, [tool("node"), tool("pnpm")]);
    assert!(Store::inspect(repo.path()).unwrap().is_some());
    assert!(store.current().unwrap().is_some());
    assert!(matches!(
        Store::inspect_with(repo.path(), Verification::Full),
        Err(Error::DamagedContents)
    ));
}

#[test]
fn a_generation_with_a_leftover_stamps_file_stays_valid() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[tool("node")], populate).unwrap();
    let (generation, _) = selected(&store);
    fs::write(generation.join("stamps.json"), b"{}").unwrap();

    assert!(shallow(repo.path()).unwrap().is_some());
    assert!(
        Store::inspect_with(repo.path(), Verification::Full)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store.reconcile(&[tool("node")], populate).unwrap(),
        Outcome::Unchanged
    );
    store
        .reconcile(&[tool("node"), tool("pnpm")], populate)
        .unwrap();
    store.remove_inactive(Duration::ZERO).unwrap();
    assert!(!generation.exists());
}

#[test]
fn a_damaged_reused_tree_is_staged_afresh_in_a_new_generation() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store
        .reconcile(&[tool("node"), tool("pnpm")], populate)
        .unwrap();
    let (generation, _) = selected(&store);
    let resource = generation.join("tools/node/resource");
    fs::remove_file(&resource).unwrap();
    fs::write(&resource, b"tampered").unwrap();
    assert_eq!(
        store
            .reconcile(&[tool("node"), tool("pnpm")], |_, _| panic!("unchanged"))
            .unwrap(),
        Outcome::Unchanged
    );

    let mut staged = Vec::new();
    let outcome = store
        .reconcile(&[tool("node")], |tool, root| {
            staged.push(tool.id.clone());
            populate(tool, root)
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Replaced);
    assert_eq!(staged, ["node"], "the damaged copy was staged, not reused");
    let (repaired, _) = selected(&store);
    assert_eq!(
        fs::read(repaired.join("tools/node/resource")).unwrap(),
        b"adjacent resource"
    );
    assert!(
        Store::inspect_with(repo.path(), Verification::Full)
            .unwrap()
            .is_some()
    );
}

#[test]
fn reusable_trees_are_hashed_only_when_the_cohort_changes() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let both = [tool("node"), tool("pnpm")];
    store.reconcile(&both, populate).unwrap();
    let (generation, _) = selected(&store);
    let resource = generation.join("tools/node/resource");
    fs::remove_file(&resource).unwrap();
    fs::write(&resource, b"tampered").unwrap();

    let node = tool("node");
    assert!(store.reusable_tree(&node, &both).unwrap().is_some());
    assert!(
        store
            .reusable_tree(&node, std::slice::from_ref(&node))
            .unwrap()
            .is_none()
    );
    let pnpm = tool("pnpm");
    assert!(
        store
            .reusable_tree(&pnpm, std::slice::from_ref(&pnpm))
            .unwrap()
            .is_some()
    );
}

#[test]
fn force_repairs_damage_an_unchanged_cohort_keeps() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [tool("node")];
    store.reconcile(&desired, populate).unwrap();
    let (generation, _) = selected(&store);
    let resource = generation.join("tools/node/resource");
    fs::remove_file(&resource).unwrap();
    fs::write(&resource, b"tampered").unwrap();
    assert!(matches!(
        Store::inspect_with(repo.path(), Verification::Full),
        Err(Error::DamagedContents)
    ));
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
fn shallow_checks_reject_broken_exports() {
    let outside = tempfile::tempdir().unwrap();
    let foreign = outside.path().join("tool");
    fs::write(&foreign, b"foreign").unwrap();
    fs::set_permissions(&foreign, fs::Permissions::from_mode(0o755)).unwrap();
    for change in 0..5 {
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        store.reconcile(&[tool("node")], populate).unwrap();
        let (generation, _) = selected(&store);
        let executable = generation.join("tools/node/bin/tool");
        let shim = generation.join("bin/node");
        match change {
            0 => fs::remove_file(&executable).unwrap(),
            1 => {
                fs::remove_file(&executable).unwrap();
                symlink(&foreign, &executable).unwrap();
            }
            2 => {
                let mode = fs::Permissions::from_mode(0o644);
                fs::set_permissions(&executable, mode).unwrap();
            }
            3 => fs::remove_file(&shim).unwrap(),
            _ => {
                fs::remove_file(&shim).unwrap();
                symlink(&foreign, &shim).unwrap();
            }
        }
        assert!(shallow(repo.path()).is_err(), "{change}");
    }
}

#[test]
fn shallow_checks_reject_a_manifest_naming_another_generation() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[tool("node")], populate).unwrap();
    let mut swapped: serde_json::Value = serde_json::from_slice(&manifest(&store)).unwrap();
    store
        .reconcile(&[tool("node"), tool("pnpm")], populate)
        .unwrap();
    let (_, inventory) = selected(&store);
    swapped["generation"] = inventory.generation.into();
    fs::write(
        store.root.join("manifest.json"),
        serde_json::to_vec(&swapped).unwrap(),
    )
    .unwrap();

    assert!(matches!(shallow(repo.path()), Err(Error::InvalidInventory)));
}
