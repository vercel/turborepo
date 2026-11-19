#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    cell::Cell,
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};

use super::*;

mod publish;
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

/// On Windows, `link.exe` cannot open a file whose path has more than 259
/// characters. Rust's MSVC targets link with it by default, and it opens the
/// standard library in the toolchain's tree.
#[test]
fn a_tree_leaves_room_for_the_rust_standard_library_on_windows() {
    // The longest file that `link.exe` opens in a nightly toolchain.
    const LIBRARY: &str = "toolchains/nightly-2026-07-03-x86_64-pc-windows-msvc/lib/rustlib/\
                           x86_64-pc-windows-msvc/lib/\
                           librustc_std_workspace_alloc-2d73472aa3fb9785.rlib";
    let f = fixture();
    let (repo, rust) = (repo(), tool("rust"));
    ensure(
        &f.store,
        repo.path(),
        std::slice::from_ref(&rust),
        &Cell::new(0),
    );
    let tree = f.store.sealed_tree(&rust).unwrap().unwrap();
    // The default store for the longest local Windows user name, 20
    // characters. Only the lengths matter, so this runs on Unix.
    let library = Path::new("C:/Users/aaaaaaaaaaaaaaaaaaaa/AppData/Local/turborepo/tools")
        .join(tree.strip_prefix(f.store.root()).unwrap())
        .join(LIBRARY);
    let length = library.as_os_str().len();
    assert!(length <= 259, "the path has {length} characters");
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

/// Another account that can write the checkout root renames `.turbo` and puts
/// a link to the victim's data in its place after setup checked it. Cleanup
/// and the editor link stay in the `.turbo` setup checked.
#[test]
fn a_turbo_swapped_after_the_check_redirects_neither_cleanup_nor_the_editor_link() {
    let repo = repo();
    let turbo = repo.path().join(".turbo");
    fs::create_dir_all(turbo.join("tools/generation-old")).unwrap();
    let victim = tempfile::tempdir().unwrap();
    fs::create_dir_all(victim.path().join("tools/private")).unwrap();
    fs::write(victim.path().join("tools/private/record"), "keep").unwrap();

    let checked = repo_turbo::RepoTurbo::open(repo.path()).unwrap();
    let moved = repo.path().join("moved");
    fs::rename(&turbo, &moved).unwrap();
    symlink(victim.path(), &turbo).unwrap();

    checked.remove_legacy_tools();
    checked.link_editor(Path::new("/stable/bin")).unwrap();
    assert_eq!(
        fs::read(victim.path().join("tools/private/record")).unwrap(),
        b"keep"
    );
    assert!(fs::symlink_metadata(victim.path().join("bin")).is_err());
    assert!(fs::symlink_metadata(moved.join("tools")).is_err());
    assert_eq!(
        fs::read_link(moved.join("bin")).unwrap(),
        Path::new("/stable/bin")
    );
}

#[test]
fn cleanup_unlinks_links_inside_tools_without_following_them() {
    let repo = repo();
    let tools = repo.path().join(".turbo/tools");
    fs::create_dir_all(tools.join("nested")).unwrap();
    let victim = tempfile::tempdir().unwrap();
    fs::write(victim.path().join("record"), "keep").unwrap();
    symlink(victim.path(), tools.join("dir-link")).unwrap();
    symlink(victim.path(), tools.join("nested/dir-link")).unwrap();
    symlink(victim.path().join("record"), tools.join("file-link")).unwrap();

    repo_turbo::RepoTurbo::open(repo.path())
        .unwrap()
        .remove_legacy_tools();
    assert!(fs::symlink_metadata(&tools).is_err());
    assert_eq!(fs::read(victim.path().join("record")).unwrap(), b"keep");
}

#[test]
fn a_turbo_that_is_a_link_or_world_writable_is_refused() {
    let repo = repo();
    let turbo = repo.path().join(".turbo");
    let victim = tempfile::tempdir().unwrap();
    fs::create_dir(victim.path().join("tools")).unwrap();
    symlink(victim.path(), &turbo).unwrap();
    assert!(matches!(
        repo_turbo::RepoTurbo::open(repo.path()),
        Err(Error::UnsafePath)
    ));
    assert!(victim.path().join("tools").is_dir());

    fs::remove_file(&turbo).unwrap();
    fs::create_dir(&turbo).unwrap();
    fs::set_permissions(&turbo, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        repo_turbo::RepoTurbo::open(repo.path()),
        Err(Error::UnsafePath)
    ));
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
fn a_missing_exported_executable_fails_activation_and_plain_setup_restages_it() {
    let f = fixture();
    let repo = repo();
    let tools = vec![tool("node"), tool("pnpm")];
    ensure(&f.store, repo.path(), &tools, &Cell::new(0));
    let selection = Selection::new(tools.clone()).unwrap();
    let root = fs::canonicalize(repo.path()).unwrap();
    let bin = f.store.sealed_tree(&tools[0]).unwrap().unwrap().join("bin");
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_file(bin.join("tool")).unwrap();

    assert!(matches!(
        f.store.activate(&root, selection.key()),
        Err(Error::Damaged(tool)) if tool == "node 1.2.3"
    ));
    assert!(f.store.sealed_tree(&tools[0]).unwrap().is_none());
    assert!(f.store.sealed_tree(&tools[1]).unwrap().is_some());

    let staged = Cell::new(0);
    ensure(&f.store, repo.path(), &tools, &staged);
    assert_eq!(staged.get(), 1);
    f.store.activate(&root, selection.key()).unwrap();
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
    for (kind, expected) in [("trees", 1), ("envs", 1), ("tmp", 0)] {
        let found = fs::read_dir(f.store.root().join(kind)).unwrap().count();
        assert_eq!(found, expected, "{kind}");
    }
    f.store.check(&Selection::new(tools).unwrap()).unwrap();
    for repo in &repos {
        let root = fs::canonicalize(repo.path()).unwrap();
        let activation = f.store.activate(&root, &keys[0]).unwrap();
        let output = Command::new(activation.stable_bin().join("node"))
            .output()
            .unwrap();
        assert!(output.status.success());
    }
}

/// What a launcher sets `DYLD_FALLBACK_LIBRARY_PATH` to. macOS removes
/// `DYLD_*` variables when a system program such as `/bin/sh` starts, so a
/// fixture program cannot report it. This evaluates everything but the
/// launcher's final `exec` in one shell instead.
fn dyld_fallback(launcher: &Path, home: &str) -> String {
    let script = r#"eval "$(sed '$d' "$0")"; printf %s "${DYLD_FALLBACK_LIBRARY_PATH-unset}""#;
    let output = Command::new("/bin/sh")
        .args(["-c", script])
        .arg(launcher)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn a_launcher_puts_its_fallback_directory_before_the_dyld_defaults() {
    let f = fixture();
    let repo = repo();
    let launch = |dyld_fallback: Option<&str>| {
        Export::Launch(Launch {
            program: TreeRef::new("runner", "bin/tool"),
            args: Vec::new(),
            dyld_fallback: dyld_fallback.map(Into::into),
        })
    };
    let mut runner = tool("runner");
    runner.executables = BTreeMap::from([("run".into(), launch(Some("lib")))]);
    ensure(&f.store, repo.path(), &[runner.clone()], &Cell::new(0));
    let launcher = fs::canonicalize(repo.path().join(".turbo/bin/run")).unwrap();
    let lib = f
        .store
        .root()
        .join("trees")
        .join(TreeSpecKey::of(&runner))
        .join("root/lib");
    let lib = lib.display();
    // dyld uses its default directories only while the variable is unset, so
    // the launcher names them after its own.
    assert_eq!(
        dyld_fallback(&launcher, "/home/me"),
        format!("{lib}:/home/me/lib:/usr/local/lib:/usr/lib")
    );
    // The value is a `:`-separated list. A home or a store with a colon would
    // name directories that neither is.
    assert_eq!(
        dyld_fallback(&launcher, "/home/a:b"),
        format!("{lib}:/usr/local/lib:/usr/lib")
    );
    let colon = tempfile::tempdir().unwrap();
    let bin = colon.path().join("a:b/envs/key/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::copy(&launcher, bin.join("run")).unwrap();
    assert_eq!(dyld_fallback(&bin.join("run"), "/home/me"), "unset");

    // A launcher without one serializes as before, so its environment keeps
    // its key and needs no new setup.
    assert!(
        !serde_json::to_string(&launch(None))
            .unwrap()
            .contains("dyld_fallback")
    );
    // The directory goes into a script, so it needs the script-safe spelling.
    runner.executables = BTreeMap::from([("run".into(), launch(Some("li'b")))]);
    assert!(matches!(
        Selection::new(vec![runner]),
        Err(Error::UnsafePath)
    ));
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
            dyld_fallback: None,
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
    // Never a Cargo home whose private parent setup has not created: Cargo
    // would create both for every user to read.
    fs::remove_file(f.store.cargo_home(&root)).unwrap();
    fs::remove_dir(f.store.cargo_homes()).unwrap();
    assert!(matches!(
        f.store.activate(&root, selection.key()),
        Err(Error::NotInstalled(_))
    ));
    // Nor one whose parent other users can read.
    fs::create_dir(f.store.cargo_homes()).unwrap();
    fs::set_permissions(f.store.cargo_homes(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        f.store.activate(&root, selection.key()),
        Err(Error::UnsafePath)
    ));
    let setup = f.store.begin_setup(repo.path(), Reuse::Sealed);
    assert!(matches!(setup, Err(Error::UnsafePath)));
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

#[test]
fn setup_moves_a_cargo_home_out_of_the_repository_directory() {
    let f = fixture();
    let repo = repo();
    let root = fs::canonicalize(repo.path()).unwrap();
    let (old, new) = (
        f.store.repo(&root).join("cargo-home"),
        f.store.cargo_home(&root),
    );
    let setup = || drop(f.store.begin_setup(repo.path(), Reuse::Sealed).unwrap());
    let config = |home: &Path| fs::read_to_string(home.join("config.toml")).ok();

    // Its configuration, such as an offline policy, stays in use.
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("config.toml"), "[net]\noffline = true\n").unwrap();
    setup();
    assert_eq!(config(&new).as_deref(), Some("[net]\noffline = true\n"));
    assert!(!old.exists());

    // A Cargo home that is already in place wins.
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("config.toml"), "").unwrap();
    setup();
    assert_eq!(config(&new).as_deref(), Some("[net]\noffline = true\n"));
    assert_eq!(config(&old).as_deref(), Some(""));

    // A link is never followed.
    let elsewhere = tempfile::tempdir().unwrap();
    fs::remove_dir_all(&old).unwrap();
    fs::remove_dir_all(&new).unwrap();
    symlink(elsewhere.path(), &old).unwrap();
    setup();
    assert!(!new.exists());
}
