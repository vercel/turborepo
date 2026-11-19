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
    assert_eq!(store.repo(&root), store.root().join("repos").join(&name));
    assert_eq!(
        store.cargo_home(&root),
        store.root().join("cargo").join(&name[..32])
    );
    assert!(
        !store.repos().exists() && !store.cargo_homes().exists(),
        "locating creates no repository state"
    );
}

/// On Windows, libgit2 checks out a Git dependency at
/// `<CARGO_HOME>/git/checkouts/<name>-<16 hex>/<7 hex>/.git/` and reserves 68
/// more characters for its longest internal file. It rejects the checkout
/// with "path too long" when the total exceeds `MAX_PATH`, 260, so the Cargo
/// home and the dependency's repository name share 146 characters.
#[test]
fn the_cargo_home_leaves_room_for_git_dependencies_on_windows() {
    const CARGO_HOME_AND_NAME: usize =
        260 - 68 - "/git/checkouts/-0123456789abcdef/0123456/.git/".len();
    assert_eq!(CARGO_HOME_AND_NAME, 146);

    // The default store for the longest local Windows user name, 20
    // characters. Only the lengths matter, so this runs on every platform.
    let store = UserStore {
        root: PathBuf::from("C:/Users/aaaaaaaaaaaaaaaaaaaa/AppData/Local/turborepo/tools"),
    };
    let cargo_home = store.cargo_home(Path::new("C:/repository"));
    let name = CARGO_HOME_AND_NAME.saturating_sub(cargo_home.as_os_str().len());
    assert!(
        name >= 40,
        "a Git dependency's repository name can be at most {name} characters"
    );
}

#[test]
fn readers_find_an_existing_store_and_never_create_one() {
    let (home, repo) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let path = home.path().join("data").join("tools");
    assert_eq!(UserStore::existing(&path, repo.path()).unwrap(), None);
    assert!(!home.path().join("data").exists());
    // A missing store is still checked exactly like `at` checks it.
    for unsafe_path in [PathBuf::from("store"), repo.path().join(".turbo/store")] {
        assert!(matches!(
            UserStore::existing(&unsafe_path, repo.path()),
            Err(Error::UnsafeStore(_))
        ));
    }
    let store = UserStore::at(&path, repo.path()).unwrap();
    assert_eq!(
        UserStore::existing(&path, repo.path()).unwrap(),
        Some(store)
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
