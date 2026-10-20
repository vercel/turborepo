#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Native Windows exports: `.exe` hard links and `.cmd` forwarders resolved
//! through PATH/PATHEXT, NTFS directory pinning, and interrupted promotion.

use std::process::{Command, Output};

use super::*;

mod stamps;

fn node() -> Tool {
    Tool {
        id: "node".into(),
        version: "24.0.0".into(),
        platform: "windows-x64".into(),
        artifact_sha256: "a".repeat(64),
        executables: BTreeMap::from([
            ("node".into(), "node.exe".into()),
            ("npm".into(), "npm.cmd".into()),
        ]),
    }
}

fn pnpm() -> Tool {
    Tool {
        id: "pnpm".into(),
        version: "10.0.0".into(),
        platform: "windows-x64".into(),
        artifact_sha256: "b".repeat(64),
        executables: BTreeMap::from([("pnpm".into(), "launch/pnpm.cmd".into())]),
    }
}

fn system32() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32")
}

// A real PE image stands in for node.exe so PATH execution is observable.
fn populate(tool: &Tool, root: &Path) -> Result<(), Error> {
    if tool.id == "node" {
        fs::copy(system32().join("cmd.exe"), root.join("node.exe"))?;
        fs::write(root.join("npm.cmd"), "@echo npm %*\r\n@exit /b 7\r\n")?;
        fs::write(root.join("resource"), tool.version.as_bytes())?;
    } else {
        fs::create_dir(root.join("launch"))?;
        fs::write(
            root.join("launch/pnpm.cmd"),
            "@echo pnpm %*\r\n@exit /b 9\r\n",
        )?;
    }
    Ok(())
}

fn manifest(store: &Store) -> Vec<u8> {
    fs::read(store.root.join("manifest.json")).unwrap()
}

// PATH consumers need the ordinary spelling: cmd.exe does not normalize
// `%~dp0..` under a verbatim `\\?\` prefix.
fn simplified(path: &Path) -> PathBuf {
    let path = path.to_str().unwrap();
    PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(path))
}

fn shell(bin: &Path, line: &str) -> Output {
    let path = std::env::join_paths([simplified(bin), system32()]).unwrap();
    Command::new(system32().join("cmd.exe"))
        .args(["/d", "/c", line])
        .env("PATH", path)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .output()
        .unwrap()
}

#[test]
fn exports_resolve_through_pathext_and_repeat_is_unchanged() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [node(), pnpm()];
    assert_eq!(
        store.reconcile(&desired, populate).unwrap(),
        Outcome::Replaced
    );
    let current = store.current().unwrap().unwrap();
    let tree = current.tool_tree(&desired[0]).unwrap();
    let names: BTreeSet<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["node.exe", "npm.cmd", "pnpm.cmd"].map(String::from))
    );
    assert!(same_file::is_same_file(current.bin.join("node.exe"), tree.join("node.exe")).unwrap());
    assert_eq!(
        fs::read(current.bin.join("pnpm.cmd")).unwrap(),
        b"@\"%~dp0..\\tools\\pnpm\\launch\\pnpm.cmd\" %*\r\n"
    );

    // Batch forwarders preserve arguments and exit status through cmd.exe.
    for (line, stdout, code) in [("npm a b", "npm a b", 7), ("pnpm c", "pnpm c", 9)] {
        let output = shell(&current.bin, line);
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), stdout);
    }
    assert_eq!(
        shell(&current.bin, "node /d /c exit 4").status.code(),
        Some(4)
    );
    // Direct process creation finds the .exe export without a shell.
    let status = Command::new("node")
        .env("PATH", simplified(&current.bin))
        .args(["/d", "/c", "exit 5"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(5));

    let before = manifest(&store);
    assert_eq!(
        store
            .reconcile(&desired, |_, _| panic!("healthy repeat must not stage"))
            .unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(manifest(&store), before);
    drop(store);
    let inspected = Store::inspect(repo.path()).unwrap().unwrap();
    assert_eq!(inspected.tools, current.tools);
    assert_eq!(inspected.bin, current.bin);
}

#[test]
fn damaged_or_foreign_exports_are_not_current_and_repair_atomically() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [node(), pnpm()];
    for damage in 0..5 {
        store.reconcile(&desired, populate).unwrap();
        let before = manifest(&store);
        let bin = store.current().unwrap().unwrap().bin;
        match damage {
            0 => {
                // Identical bytes are not the verified tree's file.
                fs::remove_file(bin.join("node.exe")).unwrap();
                fs::copy(bin.join("../tools/node/node.exe"), bin.join("node.exe")).unwrap();
            }
            1 => fs::write(bin.join("npm.cmd"), "@echo forged\r\n").unwrap(),
            2 => fs::write(bin.join("foreign.cmd"), "@echo foreign\r\n").unwrap(),
            3 => fs::remove_file(bin.join("pnpm.cmd")).unwrap(),
            _ => {
                fs::remove_file(bin.join("pnpm.cmd")).unwrap();
                fs::create_dir(bin.join("pnpm.cmd")).unwrap();
            }
        }
        assert!(store.current().is_err(), "damage {damage}");
        assert!(!store.is_current(&desired).unwrap());
        assert!(!store.can_reuse(&desired[0]).unwrap());
        assert_eq!(manifest(&store), before);
        let mut calls = Vec::new();
        store
            .reconcile(&desired, |tool, root| {
                calls.push(tool.id.clone());
                populate(tool, root)
            })
            .unwrap();
        assert_eq!(calls, ["node", "pnpm"]);
        assert!(store.is_current(&desired).unwrap());
    }
}

#[test]
fn unsupported_export_types_fail_before_publication() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    store.reconcile(&[node()], populate).unwrap();
    let before = manifest(&store);
    for path in ["bin/tool", "tool.ps1", "bad%PATH%.cmd", "bad!x.cmd"] {
        let mut tool = pnpm();
        tool.executables = BTreeMap::from([("pnpm".into(), path.into())]);
        let result = store.reconcile(&[node(), tool], |tool, root| {
            if tool.id == "node" {
                return populate(tool, root);
            }
            let file = root.join(relative_path(path));
            fs::create_dir_all(file.parent().unwrap())?;
            fs::write(file, "@exit /b 1\r\n")?;
            Ok(())
        });
        assert!(
            matches!(result, Err(Error::UnsupportedPlatform)),
            "{path}: {result:?}"
        );
        assert_eq!(manifest(&store), before);
    }
}

#[test]
fn ntfs_pins_selected_directories_and_serializes_writers() {
    let repo = tempfile::tempdir().unwrap();
    let store = Store::open(repo.path()).unwrap();
    assert!(matches!(Store::open(repo.path()), Err(Error::Busy)));
    let tools = repo.path().join(".turbo/tools");
    let moved = repo.path().join(".turbo/moved");
    assert!(fs::rename(&tools, &moved).is_err());
    let outside = tempfile::tempdir().unwrap();
    assert!(fs::rename(repo.path(), outside.path().join("moved-repo")).is_err());
    drop(store);
    fs::rename(&tools, &moved).unwrap();
    fs::rename(&moved, &tools).unwrap();
    drop(Store::open(repo.path()).unwrap());
}

#[test]
fn abrupt_exit_before_publication_keeps_selection_and_releases_lock() {
    let repo = tempfile::tempdir().unwrap();
    let before = {
        let mut store = Store::open(repo.path()).unwrap();
        store.reconcile(&[node()], populate).unwrap();
        manifest(&store)
    };
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "windows_tests::crash_child", "--ignored"])
        .env("TURBO_INSTALL_CRASH_REPO", repo.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(79));
    let mut store = Store::open(repo.path()).unwrap();
    assert_eq!(manifest(&store), before);
    let current = store.current().unwrap().unwrap();
    assert_eq!(current.tools, [node()]);
    assert_eq!(
        fs::read(current.tool_tree(&node()).unwrap().join("resource")).unwrap(),
        b"24.0.0"
    );
    let mut next = node();
    next.version = "24.1.0".into();
    assert_eq!(
        store.reconcile(&[next.clone()], populate).unwrap(),
        Outcome::Replaced
    );
    assert!(store.is_current(&[next]).unwrap());
}

#[test]
#[ignore = "subprocess fixture for abrupt interruption without destructors"]
fn crash_child() {
    let repo = std::env::var_os("TURBO_INSTALL_CRASH_REPO").expect("child fixture path");
    let mut store = Store::open(Path::new(&repo)).unwrap();
    let mut next = node();
    next.version = "24.1.0".into();
    store
        .reconcile(&[next], |tool, root| {
            populate(tool, root)?;
            std::process::exit(79);
        })
        .unwrap();
}
