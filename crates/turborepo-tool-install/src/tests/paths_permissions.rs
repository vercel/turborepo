use super::*;

fn node_fixture(_: &Tool, root: &Path) -> Result<(), Error> {
    for (path, bytes, mode) in [
        ("bin/node", b"#!/bin/sh\nexit 0\n".as_slice(), 0o755),
        ("lib/node_modules/npm/bin/npm-cli.js", b"npm fixture", 0o755),
        ("lib/node_modules/npm/bin/npx-cli.js", b"npx fixture", 0o755),
        (
            "lib/node_modules/npm/node_modules/@npmcli/config/package.json",
            b"{\"name\":\"@npmcli/config\"}",
            0o644,
        ),
        (
            "lib/node_modules/npm/node_modules/@npmcli/config/lib/index.js",
            b"scoped runtime resource",
            0o644,
        ),
        (
            "lib/node_modules/npm/node_modules/@isaacs/cliui/LICENSE.txt",
            b"scoped license",
            0o644,
        ),
        (
            "lib/node_modules/npm/.npmrc",
            b"hidden configuration",
            0o644,
        ),
        (
            "lib/node_modules/npm/node_modules/@npmcli/config/Bin/Helper",
            b"#!/bin/sh\nexit 0\n",
            0o700,
        ),
        ("include/node/v8-array-buffer.h", b"header fixture", 0o644),
        ("share/man/man1/node.1", b"manual fixture", 0o644),
        ("LICENSE", b"license fixture", 0o644),
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().ok_or(Error::UnsafePath)?)?;
        fs::write(&path, bytes)?;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    symlink(
        "../lib/node_modules/npm/bin/npm-cli.js",
        root.join("bin/npm"),
    )?;
    symlink(
        "../lib/node_modules/npm/bin/npx-cli.js",
        root.join("bin/npx"),
    )?;
    Ok(())
}

#[test]
fn scoped_node_tree_preserves_shims_resources_no_op_and_reuse() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let mut node = tool("node");
    node.executables = BTreeMap::from([
        ("node".into(), "bin/node".into()),
        ("npm".into(), "bin/npm".into()),
        ("npx".into(), "bin/npx".into()),
        (
            "helper".into(),
            "lib/node_modules/npm/node_modules/@npmcli/config/Bin/Helper".into(),
        ),
    ]);
    assert_eq!(
        store.reconcile(&[node.clone()], node_fixture).unwrap(),
        Outcome::Replaced
    );
    let before = manifest(&store);
    let bin = store.current().unwrap().unwrap().bin;
    assert_eq!(fs::read(bin.join("npm")).unwrap(), b"npm fixture");
    assert_eq!(fs::read(bin.join("npx")).unwrap(), b"npx fixture");
    assert!(Command::new(bin.join("node")).status().unwrap().success());
    assert!(Command::new(bin.join("helper")).status().unwrap().success());
    assert_eq!(
        fs::read_link(bin.join("npm")).unwrap(),
        Path::new("../tools/node/bin/npm")
    );
    assert_eq!(
        store
            .reconcile(&[node.clone()], |_, _| panic!(
                "healthy repeat must not stage"
            ))
            .unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(manifest(&store), before);
    assert_eq!(store.current().unwrap().unwrap().bin, bin);

    // A changed inventory reuses the full node tree, including relative links,
    // while only the new tool goes through the callback.
    let pnpm = tool("pnpm");
    let mut staged = Vec::new();
    assert_eq!(
        store
            .reconcile(&[pnpm.clone(), node.clone()], |tool, root| {
                staged.push(tool.id.clone());
                populate(tool, root)
            })
            .unwrap(),
        Outcome::Replaced
    );
    assert_eq!(staged, ["pnpm"]);
    let current = store.current().unwrap().unwrap();
    let tree = current.bin.join("../tools/node");
    for (path, bytes) in [
        (
            "lib/node_modules/npm/node_modules/@npmcli/config/lib/index.js",
            b"scoped runtime resource".as_slice(),
        ),
        (
            "lib/node_modules/npm/node_modules/@isaacs/cliui/LICENSE.txt",
            b"scoped license",
        ),
        ("lib/node_modules/npm/.npmrc", b"hidden configuration"),
        ("LICENSE", b"license fixture"),
    ] {
        assert_eq!(fs::read(tree.join(path)).unwrap(), bytes);
    }
    assert_eq!(
        fs::read_link(tree.join("bin/npm")).unwrap(),
        Path::new("../lib/node_modules/npm/bin/npm-cli.js")
    );
    assert_eq!(fs::read(current.bin.join("npm")).unwrap(), b"npm fixture");
    assert_eq!(
        store
            .reconcile(&[node], |_, _| panic!("reuse must not stage"))
            .unwrap(),
        Outcome::Replaced
    );
    let current = store.current().unwrap().unwrap();
    assert!(!current.bin.join("pnpm").exists());
    assert_eq!(fs::read(current.bin.join("npx")).unwrap(), b"npx fixture");
}

#[test]
fn artifact_spelling_and_bookkeeping_identity_have_distinct_rules() {
    for name in [
        "@npmcli",
        "@isaacs",
        "LICENSE",
        ".npmrc",
        ".package-lock.json",
        "a+b",
        "a b",
    ] {
        assert!(artifact_component(name).is_ok(), "{name}");
    }
    assert!(relative("lib/node_modules/@npmcli/config/Bin/Helper").is_ok());
    for bad in [
        "",
        ".",
        "..",
        "NUL.txt",
        "CON .txt",
        "COM1",
        "lPt9.log",
        "CLOCK$",
        "CONIN$",
        "CONOUT$",
        "a/b",
        "a\\b",
        "C:tool",
        "tool:stream",
        "tool.",
        "tool ",
        "PROGRA~1",
        "a\0b",
        "a\nb",
        "é",
        "a?b",
        "a*b",
    ] {
        assert!(artifact_component(bad).is_err(), "{bad:?}");
    }
    assert!(artifact_component(&"a".repeat(256)).is_err());
    for bad in [
        "../tool",
        "/bin/tool",
        "bin//tool",
        "bin/./tool",
        "bin/../tool",
        "bin/tool:stream",
        "bin/NUL.txt",
    ] {
        assert!(relative(bad).is_err(), "{bad}");
    }
    assert!(relative(&"a/".repeat(2049)).is_err());
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    for bad in ["@npmcli", "Node", "a+b", ".node", "node "] {
        let mut desired = tool("node");
        desired.id = bad.into();
        assert!(
            store
                .reconcile(&[desired], |_, _| panic!("invalid ID"))
                .is_err()
        );
        let mut desired = tool("node");
        desired.executables = BTreeMap::from([(bad.into(), "bin/tool".into())]);
        assert!(
            store
                .reconcile(&[desired], |_, _| panic!("invalid shim name"))
                .is_err()
        );
    }
    assert!(store.current().unwrap().is_none());
}

#[test]
fn unsafe_artifact_names_and_escaping_links_preserve_selected_generation() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let mut desired = tool("node");
    store.reconcile(&[desired.clone()], populate).unwrap();
    let before = manifest(&store);
    desired.version = "2.0.0".into();
    for bad in [
        "NUL.txt",
        "CON .txt",
        "tool:stream",
        "tool.",
        "a\\b",
        "PROGRA~1",
    ] {
        assert!(matches!(
            store.reconcile(&[desired.clone()], |tool, root| {
                populate(tool, root)?;
                fs::write(root.join(bad), b"unsafe artifact")?;
                Ok(())
            }),
            Err(Error::UnsafePath)
        ));
        assert_eq!(manifest(&store), before);
    }
    assert!(matches!(
        store.reconcile(&[desired], |tool, root| {
            populate(tool, root)?;
            symlink(repo.path(), root.join("@scope"))?;
            Ok(())
        }),
        Err(Error::UnsafePath)
    ));
    assert_eq!(manifest(&store), before);
    assert_eq!(
        fs::read(store.current().unwrap().unwrap().bin.join("node")).unwrap(),
        b"1.2.3"
    );
}

#[test]
fn direct_and_symlink_exports_require_owner_execute_before_publication() {
    for executable in ["bin/tool", "bin/alias"] {
        let repo = tempfile::tempdir().unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        let mut desired = tool("node");
        desired.executables.insert("node".into(), executable.into());
        store.reconcile(&[desired.clone()], populate).unwrap();
        let before = manifest(&store);
        desired.version = "2.0.0".into();
        for mode in [0o600, 0o601, 0o610, 0o611, 0o645, 0o654] {
            assert!(
                matches!(
                    store.reconcile(&[desired.clone()], |tool, root| {
                        populate(tool, root)?;
                        fs::set_permissions(
                            root.join("bin/tool"),
                            fs::Permissions::from_mode(mode),
                        )?;
                        Ok(())
                    }),
                    Err(Error::InvalidInventory)
                ),
                "{mode:o}"
            );
            assert_eq!(manifest(&store), before);
            assert!(store.current().is_ok());
        }
        for mode in [0o700, 0o701, 0o710, 0o755] {
            desired.version = format!("2.{mode:o}");
            assert_eq!(
                store
                    .reconcile(&[desired.clone()], |tool, root| {
                        populate(tool, root)?;
                        fs::set_permissions(
                            root.join("bin/tool"),
                            fs::Permissions::from_mode(mode),
                        )?;
                        Ok(())
                    })
                    .unwrap(),
                Outcome::Replaced
            );
            let before = manifest(&store);
            assert_eq!(
                store
                    .reconcile(&[desired.clone()], |_, _| panic!("healthy repeat"))
                    .unwrap(),
                Outcome::Unchanged
            );
            assert_eq!(manifest(&store), before);
        }
    }
}

#[test]
fn legacy_non_owner_executable_inventory_is_not_current_or_a_no_op() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = tool("node");
    store
        .reconcile(std::slice::from_ref(&desired), populate)
        .unwrap();
    let mut inventory: Inventory = serde_json::from_slice(&manifest(&store)).unwrap();
    let tree = store.root.join(&inventory.generation).join("tools/node");
    fs::set_permissions(tree.join("bin/tool"), fs::Permissions::from_mode(0o601)).unwrap();
    // Model inventory published by the old validator: hash and bytes agree,
    // but the installation owner still cannot execute the mapped file.
    inventory.tools[0].tree_sha256 = tree_hash(&tree).unwrap();
    fs::write(
        store.root.join("manifest.json"),
        serde_json::to_vec(&inventory).unwrap(),
    )
    .unwrap();
    assert!(matches!(store.current(), Err(Error::InvalidInventory)));
    let mut stages = 0;
    assert_eq!(
        store
            .reconcile(&[desired], |tool, root| {
                stages += 1;
                populate(tool, root)
            })
            .unwrap(),
        Outcome::Replaced
    );
    assert_eq!(stages, 1);
    assert!(store.current().is_ok());
}
