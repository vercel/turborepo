use std::time::SystemTime;

use super::*;

/// Sets up `tools` for `repo`, staging each tree with `stage`. Also returns
/// whether setup asked the caller to record an environment.
fn setup(
    store: &UserStore,
    repo: &Path,
    tools: &[Tool],
    stage: impl FnMut(&Tool, &Path) -> Result<(), Error>,
) -> (Result<Ensured, Error>, bool) {
    let selection = Selection::new(tools.to_vec()).unwrap();
    let recorded = Cell::new(false);
    let result = store
        .begin_setup(repo, Reuse::Sealed)
        .unwrap()
        .ensure(&selection, stage, |_| {
            recorded.set(true);
            Ok(())
        });
    (result, recorded.get())
}

fn count(store: &UserStore, kind: &str) -> usize {
    fs::read_dir(store.root().join(kind)).unwrap().count()
}

fn assert_nothing_published(store: &UserStore, repo: &Path) {
    for kind in ["trees", "envs", "repos", "tmp"] {
        assert_eq!(count(store, kind), 0, "{kind} is not empty");
    }
    assert!(!repo.join(".turbo").exists());
}

#[test]
fn a_staged_link_out_of_its_tree_is_rejected_and_nothing_is_published() {
    let f = fixture();
    let repo = repo();
    let (result, recorded) = setup(&f.store, repo.path(), &[tool("node")], |tool, root| {
        populate(tool, root)?;
        // An existing file beside the tree, so only containment rejects it.
        fs::write(root.join("../x"), "outside")?;
        symlink("../../x", root.join("bin/escape"))?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::UnsafePath)), "{result:?}");
    assert!(!recorded);
    assert_nothing_published(&f.store, repo.path());
}

#[test]
fn a_staged_absolute_link_is_rejected_even_into_its_own_tree() {
    let f = fixture();
    let repo = repo();
    let (result, recorded) = setup(&f.store, repo.path(), &[tool("node")], |tool, root| {
        populate(tool, root)?;
        // Dangles as soon as the tree moves out of staging.
        symlink(root.join("bin/tool"), root.join("bin/absolute"))?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::UnsafePath)), "{result:?}");
    assert!(!recorded);
    assert_nothing_published(&f.store, repo.path());
}

#[test]
fn an_export_its_owner_cannot_execute_gets_no_environment() {
    let f = fixture();
    let repo = repo();
    let tools = [tool("node")];
    let (result, recorded) = setup(&f.store, repo.path(), &tools, |tool, root| {
        populate(tool, root)?;
        fs::set_permissions(root.join("bin/tool"), fs::Permissions::from_mode(0o655))?;
        Ok(())
    });
    assert!(matches!(result, Err(Error::InvalidInventory)), "{result:?}");
    assert!(!recorded);
    assert_eq!(count(&f.store, "envs"), 0);
    assert_eq!(count(&f.store, "repos"), 0);
    assert!(!repo.path().join(".turbo").exists());
    let selection = Selection::new(tools.to_vec()).unwrap();
    assert!(matches!(
        f.store.check(&selection),
        Err(Error::NotInstalled(env)) if env.starts_with("environment ")
    ));
}

#[test]
fn an_installed_export_that_loses_owner_execute_fails_check_until_forced_setup() {
    let f = fixture();
    let repo = repo();
    let tools = vec![tool("node")];
    let ensured = ensure(&f.store, repo.path(), &tools, &Cell::new(0));
    let selection = Selection::new(tools).unwrap();
    let root = fs::canonicalize(repo.path()).unwrap();
    let bin = f
        .store
        .activate(&root, &ensured.env)
        .unwrap()
        .bin()
        .join("node");
    let file = fs::canonicalize(&bin).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o455)).unwrap();

    // Root may execute a file that grants any execute bit.
    if !rustix::process::geteuid().is_root() {
        assert!(
            Command::new(&bin).output().is_err(),
            "the export still runs"
        );
    }
    assert!(matches!(f.store.check(&selection), Err(Error::Damaged(tool)) if tool == "node 1.2.3"));
    // Activation reads seals and shims, never trees, so it does not notice.
    assert!(f.store.activate(&root, &ensured.env).is_ok());

    let staged = Cell::new(0);
    f.store
        .begin_setup(repo.path(), Reuse::Verified)
        .unwrap()
        .ensure(
            &selection,
            |tool, root| {
                staged.set(staged.get() + 1);
                populate(tool, root)
            },
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(staged.get(), 1);
    f.store.check(&selection).unwrap();
    assert!(Command::new(&bin).output().unwrap().status.success());
}

const CRASH_STORE: &str = "TURBO_TOOL_INSTALL_TEST_CRASH_STORE";
const CRASH_REPO: &str = "TURBO_TOOL_INSTALL_TEST_CRASH_REPO";
const CRASH_CODE: i32 = 86;

/// The child process of
/// `a_setup_killed_before_publishing_leaves_only_staging_for_collection`.
#[test]
#[ignore = "runs only as that test's child process"]
fn setup_exits_abruptly_while_staging() {
    let (Some(store), Some(repo)) = (std::env::var_os(CRASH_STORE), std::env::var_os(CRASH_REPO))
    else {
        return;
    };
    let store = UserStore::at(Path::new(&store), Path::new(&repo)).unwrap();
    let _ = setup(&store, Path::new(&repo), &[tool("node")], |tool, root| {
        populate(tool, root)?;
        // No destructor runs: the staging directory stays in `tmp`.
        std::process::exit(CRASH_CODE)
    });
    unreachable!("setup returned instead of exiting");
}

#[test]
fn a_setup_killed_before_publishing_leaves_only_staging_for_collection() {
    let f = fixture();
    let repo = repo();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "--ignored",
            "tests::publish::setup_exits_abruptly_while_staging",
        ])
        .env(CRASH_STORE, f.store.root())
        .env(CRASH_REPO, repo.path())
        .output()
        .unwrap()
        .status;
    assert_eq!(status.code(), Some(CRASH_CODE), "the child did not crash");
    assert_eq!(count(&f.store, "tmp"), 1);
    for kind in ["trees", "envs", "repos"] {
        assert_eq!(count(&f.store, kind), 0, "{kind} is not empty");
    }
    assert!(!repo.path().join(".turbo").exists());

    let tools = vec![tool("node")];
    let staged = Cell::new(0);
    let ensured = ensure(&f.store, repo.path(), &tools, &staged);
    assert_eq!(staged.get(), 1);
    assert_eq!(count(&f.store, "trees"), 1);
    assert_eq!(count(&f.store, "envs"), 1);
    f.store.check(&Selection::new(tools).unwrap()).unwrap();

    let collected = super::gc::collect(&f.store, &[(repo.path(), &ensured.env)], SystemTime::now());
    assert_eq!(collected.removed.entries, 0);
    assert_eq!(count(&f.store, "tmp"), 0);
    let root = fs::canonicalize(repo.path()).unwrap();
    let bin = f
        .store
        .activate(&root, &ensured.env)
        .unwrap()
        .bin()
        .join("node");
    assert!(Command::new(bin).output().unwrap().status.success());
}
