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
