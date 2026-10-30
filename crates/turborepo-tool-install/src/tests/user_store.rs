use super::*;

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn rejected(result: Result<UserStore, Error>) -> bool {
    matches!(result, Err(Error::UnsafeStore(_)))
}

#[test]
fn a_store_is_created_private_and_keys_each_repository_by_its_canonical_root() {
    let (home, repo) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = home.path().join("data").join("tools");
    let store = UserStore::at(&path, repo.path()).unwrap();
    assert_eq!(store.root(), fs::canonicalize(&path).unwrap());
    for created in [home.path().join("data"), path.clone()] {
        assert_eq!(mode(&created), 0o700, "{}", created.display());
    }
    // Locating again accepts the store it created.
    assert_eq!(UserStore::at(&path, repo.path()).unwrap(), store);

    let root = fs::canonicalize(repo.path()).unwrap();
    let name = hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()));
    let state = store.root().join("repos").join(name);
    assert_eq!(store.repo(&root), state);
    assert_eq!(store.cargo_home(&root), state.join("cargo-home"));
    assert!(
        !store.repos().exists(),
        "locating creates no repository state"
    );
}

#[test]
fn unsafe_store_paths_are_rejected_before_anything_is_created() {
    let (home, repo) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    assert!(rejected(UserStore::at(Path::new("store"), repo.path())));
    assert!(rejected(UserStore::at(
        &home.path().join("a/../store"),
        repo.path()
    )));
    assert!(!home.path().join("a").exists());

    assert!(rejected(UserStore::at(
        &repo.path().join(".turbo/store"),
        repo.path()
    )));
    assert!(!repo.path().join(".turbo").exists());
    assert!(rejected(UserStore::at(repo.path(), repo.path())));
    let outer = repo.path().parent().unwrap();
    assert!(rejected(UserStore::at(outer, repo.path())));

    let real = home.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = home.path().join("link");
    symlink(&real, &link).unwrap();
    assert!(rejected(UserStore::at(&link, repo.path())));

    // Others may read an existing store but never write it.
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(UserStore::at(&real, repo.path()).is_ok());
    fs::set_permissions(&real, fs::Permissions::from_mode(0o775)).unwrap();
    assert!(rejected(UserStore::at(&real, repo.path())));
}

#[test]
fn launchers_and_inspection_use_the_cargo_home_the_store_was_published_with() {
    let (home, repo) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut rust = tool("rust");
    rust.executables = BTreeMap::from([(
        "cargo".into(),
        Export::Launch(Launch {
            program: TreeRef::new("rust", "bin/tool"),
            args: Vec::new(),
        }),
    )]);
    rust.vars = BTreeMap::from([(ManagedVar::CargoHome, VarValue::RepoCargoHome)]);
    let desired = [rust];
    let stage = |_: &Tool, root: &Path| -> Result<(), Error> {
        fs::create_dir(root.join("bin"))?;
        fs::write(root.join("bin/tool"), "#!/bin/sh\necho \"$CARGO_HOME\"\n")?;
        fs::set_permissions(root.join("bin/tool"), fs::Permissions::from_mode(0o755))?;
        Ok(())
    };
    let cargo = |store: &Store| {
        let bin = store.current().unwrap().unwrap().bin;
        let output = Command::new(bin.join("cargo"))
            .env_clear()
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    };
    let root = fs::canonicalize(repo.path()).unwrap();

    // A Store that knows no Cargo home cannot publish a tool that needs one.
    let mut store = Store::open(repo.path()).unwrap();
    assert!(matches!(
        store.reconcile(&desired, stage),
        Err(Error::InvalidInventory)
    ));
    assert!(store.current().unwrap().is_none());
    drop(store);

    // Quotes and spaces in the store path reach the launcher intact.
    let first = UserStore::at(&home.path().join("it's a store"), repo.path()).unwrap();
    let mut store = Store::open(repo.path())
        .unwrap()
        .with_user_store(&first)
        .unwrap();
    store.reconcile(&desired, stage).unwrap();
    let expected = first.cargo_home(&root);
    assert_eq!(cargo(&store), format!("{}\n", expected.display()));
    assert_eq!(
        Store::inspect(repo.path()).unwrap().unwrap().cargo_home,
        Some(expected)
    );
    assert!(!repo.path().join(".turbo/cargo-home").exists());
    drop(store);

    // Another store republishes the launchers from the same trees.
    let second = UserStore::at(&home.path().join("second"), repo.path()).unwrap();
    let mut store = Store::open(repo.path())
        .unwrap()
        .with_user_store(&second)
        .unwrap();
    let outcome = store
        .reconcile(&desired, |_, _| Err(Error::InvalidInventory))
        .unwrap();
    assert_eq!(outcome, Outcome::Replaced);
    assert_eq!(
        cargo(&store),
        format!("{}\n", second.cargo_home(&root).display())
    );
}
