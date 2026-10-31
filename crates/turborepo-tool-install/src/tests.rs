#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    cell::Cell,
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};

use super::*;

mod gc;
mod user_store;

fn tool(id: &str) -> Tool {
    Tool {
        id: id.into(),
        version: "1.2.3".into(),
        platform: "linux-x86_64-gnu".into(),
        artifact_sha256: "a".repeat(64),
        executables: BTreeMap::from([(id.into(), "bin/tool".into())]),
        vars: Default::default(),
        staging: Staging::new("fixture", 1, &()).unwrap(),
    }
}

fn populate(tool: &Tool, root: &Path) -> Result<(), Error> {
    fs::create_dir(root.join("bin"))?;
    let script = "#!/bin/sh\necho \"$GOTOOLCHAIN|$CARGO_HOME|$*\"\n";
    fs::write(root.join("bin/tool"), script)?;
    fs::set_permissions(root.join("bin/tool"), fs::Permissions::from_mode(0o755))?;
    fs::write(root.join("resource"), &tool.id)?;
    symlink("tool", root.join("bin/alias"))?;
    Ok(())
}

struct Fixture {
    _home: tempfile::TempDir,
    store: UserStore,
}

fn fixture() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    // Locating checks containment against one repository; every test
    // repository is a separate temporary directory outside the store.
    let outside = tempfile::tempdir().unwrap();
    // Quotes and spaces in the store path must reach launchers intact.
    let store = UserStore::at(&home.path().join("it's a store"), outside.path()).unwrap();
    Fixture { _home: home, store }
}

fn repo() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Ensures `tools` for `repo`, counting how many trees were staged.
fn ensure(store: &UserStore, repo: &Path, tools: &[Tool], staged: &Cell<usize>) -> Ensured {
    let selection = Selection::new(tools.to_vec()).unwrap();
    store
        .begin_setup(repo, Reuse::Sealed)
        .unwrap()
        .ensure(
            &selection,
            |tool, root| {
                staged.set(staged.get() + 1);
                populate(tool, root)
            },
            |_| Ok(()),
        )
        .unwrap()
}

#[test]
fn a_second_repository_with_the_same_selection_stages_nothing_and_shares_the_environment() {
    let f = fixture();
    let (first, second) = (repo(), repo());
    let tools = [tool("node"), tool("pnpm")];
    let staged = Cell::new(0);
    let one = ensure(&f.store, first.path(), &tools, &staged);
    assert_eq!(staged.get(), 2);
    let two = ensure(&f.store, second.path(), &tools, &staged);
    assert_eq!(staged.get(), 2, "the second repository staged a tree");
    assert_eq!(one.env, two.env);
    assert!(two.installed.is_empty());
    // A different selection reuses the shared trees it has in common.
    let three = ensure(&f.store, second.path(), &tools[..1], &staged);
    assert_eq!(staged.get(), 2);
    assert_ne!(three.env, one.env);
}

#[test]
fn tools_that_stage_the_same_artifact_differently_never_share_a_tree() {
    let f = fixture();
    let (first, second) = (repo(), repo());
    let plain = tool("node");
    let nested = Tool {
        staging: Staging::new("fixture", 1, &serde_json::json!({"root": "pkg"})).unwrap(),
        ..tool("node")
    };
    let staged = Cell::new(0);
    let one = ensure(
        &f.store,
        first.path(),
        std::slice::from_ref(&nested),
        &staged,
    );
    let two = ensure(
        &f.store,
        second.path(),
        std::slice::from_ref(&plain),
        &staged,
    );
    assert_eq!(
        staged.get(),
        2,
        "the second repository reused the first's tree"
    );
    assert_ne!(one.env, two.env);
    let tree = |tool: &Tool| f.store.sealed_tree(tool).unwrap().unwrap();
    assert_ne!(tree(&plain), tree(&nested));

    // A tree sealed for other staging is never reused under this tool's key.
    let seal = |tool: &Tool| tree(tool).with_file_name("seal.json");
    fs::copy(seal(&nested), seal(&plain)).unwrap();
    assert!(f.store.sealed_tree(&plain).unwrap().is_none());
    ensure(
        &f.store,
        second.path(),
        std::slice::from_ref(&plain),
        &staged,
    );
    assert_eq!(staged.get(), 3, "a mismatched seal was trusted");
    assert!(f.store.sealed_tree(&plain).unwrap().is_some());
}

#[test]
fn the_repository_holds_only_an_editor_link_to_its_stable_bin() {
    let f = fixture();
    let repo = repo();
    let old = repo.path().join(".turbo/tools/generation-old/tools/node");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("file"), "old").unwrap();
    fs::set_permissions(old.join("file"), fs::Permissions::from_mode(0o444)).unwrap();
    let ensured = ensure(&f.store, repo.path(), &[tool("node")], &Cell::new(0));
    assert!(!repo.path().join(".turbo/tools").exists());
    let root = fs::canonicalize(repo.path()).unwrap();
    let activation = f.store.activate(&root, &ensured.env).unwrap();
    let editor = repo.path().join(".turbo/bin");
    assert_eq!(fs::read_link(&editor).unwrap(), activation.stable_bin());
    assert_eq!(
        fs::canonicalize(&editor).unwrap(),
        fs::canonicalize(activation.bin()).unwrap()
    );
    let output = Command::new(editor.join("node")).output().unwrap();
    assert!(output.status.success());

    // Switching environments moves the stable link, not the editor link.
    let mut other = tool("node");
    other.version = "2.0.0".into();
    let switched = ensure(&f.store, repo.path(), &[other], &Cell::new(0));
    assert_eq!(fs::read_link(&editor).unwrap(), activation.stable_bin());
    assert_eq!(
        fs::canonicalize(&editor).unwrap(),
        fs::canonicalize(f.store.activate(&root, &switched.env).unwrap().bin()).unwrap()
    );
}

#[test]
fn trees_are_read_only_and_check_finds_a_modified_file() {
    let f = fixture();
    let repo = repo();
    let tools = vec![tool("node")];
    ensure(&f.store, repo.path(), &tools, &Cell::new(0));
    let selection = Selection::new(tools).unwrap();
    f.store.check(&selection).unwrap();
    let root = fs::canonicalize(repo.path()).unwrap();
    let bin = f
        .store
        .activate(&root, selection.key())
        .unwrap()
        .bin()
        .to_owned();
    let file = fs::canonicalize(bin.join("node")).unwrap();
    let mode = fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o222, 0, "tree files are writable: {mode:o}");

    fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&file, "#!/bin/sh\necho tampered\n").unwrap();
    assert!(matches!(f.store.check(&selection), Err(Error::Damaged(tool)) if tool == "node 1.2.3"));

    // Forced setup re-verifies, restages only the damaged tree, and heals it.
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
}

#[test]
fn forced_setup_restages_a_tree_with_a_dangling_link() {
    let f = fixture();
    let repo = repo();
    let tools = vec![tool("node")];
    ensure(&f.store, repo.path(), &tools, &Cell::new(0));
    let selection = Selection::new(tools).unwrap();
    let root = fs::canonicalize(repo.path()).unwrap();
    let bin = f
        .store
        .activate(&root, selection.key())
        .unwrap()
        .bin()
        .join("node");
    let tree_bin = fs::canonicalize(&bin).unwrap();
    std::os::unix::fs::symlink("missing", tree_bin.with_file_name("dangling")).unwrap();

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
}

#[test]
fn check_reports_an_environment_that_was_never_installed() {
    let f = fixture();
    let selection = Selection::new(vec![tool("node")]).unwrap();
    assert!(matches!(
        f.store.check(&selection),
        Err(Error::NotInstalled(_))
    ));
    let root = fs::canonicalize(repo().path()).unwrap();
    assert!(matches!(
        f.store.activate(&root, selection.key()),
        Err(Error::NotInstalled(_))
    ));
}

#[test]
fn setups_racing_on_the_same_missing_tree_both_succeed_and_leave_one_tree() {
    let f = fixture();
    let repos = [repo(), repo()];
    let barrier = std::sync::Barrier::new(2);
    let tools = vec![tool("node")];
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = repos
            .iter()
            .map(|repo| {
                let (store, barrier, tools) = (&f.store, &barrier, &tools);
                scope.spawn(move || {
                    let selection = Selection::new(tools.clone()).unwrap();
                    let session = store.begin_setup(repo.path(), Reuse::Sealed).unwrap();
                    session.ensure(
                        &selection,
                        |tool, root| {
                            populate(tool, root)?;
                            // Both have staged before either publishes.
                            barrier.wait();
                            Ok(())
                        },
                        |_| Ok(()),
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let keys: Vec<_> = results.into_iter().map(|r| r.unwrap().env).collect();
    assert_eq!(keys[0], keys[1]);
    let trees = fs::read_dir(f.store.root().join("trees")).unwrap().count();
    assert_eq!(trees, 1);
    assert_eq!(fs::read_dir(f.store.root().join("tmp")).unwrap().count(), 0);
    f.store.check(&Selection::new(tools).unwrap()).unwrap();
}

#[test]
fn launchers_run_other_trees_without_an_inherited_environment_and_leave_cargo_home_to_activation() {
    let f = fixture();
    let repo = repo();
    let mut runner = tool("runner");
    runner.executables = BTreeMap::from([(
        "run".into(),
        Export::Launch(Launch {
            program: TreeRef::new("node", "bin/tool"),
            args: vec![
                LaunchArg::Tree(TreeRef::new("runner", "resource")),
                LaunchArg::Word("--flag".into()),
            ],
        }),
    )]);
    runner.vars = BTreeMap::from([
        (ManagedVar::Gotoolchain, VarValue::Word("local".into())),
        (ManagedVar::CargoHome, VarValue::RepoCargoHome),
        (ManagedVar::Goroot, VarValue::Tree(TreeRef::new("node", ""))),
        (
            ManagedVar::UvPythonInstallDir,
            VarValue::Tree(TreeRef::new("python", "")),
        ),
    ]);
    // A launcher may only name selected trees.
    assert!(matches!(
        Selection::new(vec![runner.clone()]),
        Err(Error::InvalidInventory)
    ));
    let ensured = ensure(
        &f.store,
        repo.path(),
        &[tool("node"), runner.clone()],
        &Cell::new(0),
    );
    // Run through the editor link, as an editor would.
    let output = Command::new(repo.path().join(".turbo/bin/run"))
        .arg("x y")
        .env_clear()
        .output()
        .unwrap();
    let selection = Selection::new(vec![tool("node"), runner]).unwrap();
    let root = fs::canonicalize(repo.path()).unwrap();
    let activation = f.store.activate(&root, selection.key()).unwrap();
    assert_eq!(activation.env(), &ensured.env);
    let resource = f
        .store
        .root()
        .join("trees")
        .join(TreeSpecKey::of(&tool("runner")))
        .join("root/resource");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("local||{} --flag x y\n", resource.display())
    );
    // Activation resolves every variable, including the per-repository ones.
    let vars = activation.vars();
    assert_eq!(
        vars["CARGO_HOME"],
        f.store.cargo_home(&root).into_os_string()
    );
    assert_eq!(
        vars["GOROOT"],
        f.store
            .root()
            .join("trees")
            .join(TreeSpecKey::of(&tool("node")))
            .join("root")
            .into_os_string()
    );
    let unselected = PathBuf::from(&vars["UV_PYTHON_INSTALL_DIR"]);
    assert!(unselected.starts_with(f.store.root().join("envs")));
    assert!(!unselected.exists());
    assert_eq!(activation.targets(), ["node", "run"]);
    // Never a Cargo home that is not a plain directory.
    fs::write(f.store.cargo_home(&root), "").unwrap();
    assert!(matches!(
        f.store.activate(&root, selection.key()),
        Err(Error::UnsafePath)
    ));
}

/// The tree key, as the store names directories, without exposing it.
struct TreeSpecKey;
impl TreeSpecKey {
    fn of(tool: &Tool) -> String {
        let session_free = fixture();
        let repo = repo();
        let selection = Selection::new(vec![tool.clone()]).unwrap();
        session_free
            .store
            .begin_setup(repo.path(), Reuse::Sealed)
            .unwrap()
            .ensure(&selection, populate, |_| Ok(()))
            .unwrap();
        let entry = fs::read_dir(session_free.store.root().join("trees"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        entry.file_name().into_string().unwrap()
    }
}

#[test]
fn managed_variable_names_are_their_serialized_names() {
    for var in ManagedVar::ALL {
        assert_eq!(serde_json::to_value(var).unwrap(), var.name());
        assert_eq!(ManagedVar::from_name(var.name()), Some(var));
    }
    let names: Vec<_> = ManagedVar::ALL.iter().map(|var| var.name()).collect();
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(ManagedVar::from_name("PATH"), None);
}
