//! Native Windows x64 additional tools: `.exe` and `.cmd` mappings, adjacent
//! resources, repeat, offline reuse, removal, and rollback while a managed
//! executable is running. Local fixtures only.
use super::*;

const TOOL_ZIP: &str = "/fixture-1.2.3-windows-x64.zip";
const OFFLINE: &[&str] = &["--frozen", "--tools-only", "--offline"];
// Exits 5 only when it finds its resource beside its installed location.
// (`findstr /x` would miss the unterminated last line, so match a substring.)
const LAUNCHER: &[u8] = b"@findstr /c:\"fixture resource\" \"%~dp0..\\share\\data.txt\" >nul \
                          && exit /b 5\r\n@exit /b 1\r\n";

fn tool(resource: &str) -> Vec<u8> {
    zip(&[
        ("bin/fixture.cmd", LAUNCHER.to_vec()),
        // A real PE image, as for node.exe.
        (
            "bin/fixture-exe.exe",
            fs::read(system32().join("cmd.exe")).unwrap(),
        ),
        ("share/data.txt", resource.as_bytes().to_vec()),
    ])
}

fn declare(f: &Fixture, artifact: Option<&[u8]>) {
    let mut config: Value = serde_json::from_str(ENABLED).unwrap();
    if let Some(artifact) = artifact {
        config["setup"] = json!({"additionalTools": {"fixture": {"version": "1.2.3",
            "artifacts": {"windows-x64": {
                "url": format!("https://downloads.example.test{TOOL_ZIP}"),
                "sha256": format!("{:x}", Sha256::digest(artifact)),
                "format": "zip", "rootPrefix": PREFIX,
                "executables": {"fixture": "bin/fixture.cmd", "fixture-exe": "bin/fixture-exe.exe"}}}}}});
    }
    fs::write(f.owned.root().join("turbo.json"), config.to_string()).unwrap();
}

fn exports(f: &Fixture) -> Vec<String> {
    let bin = f.plan().path_prepend().to_path_buf();
    let mut names: Vec<_> = fs::read_dir(bin)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn additional_tools_export_and_find_resources_then_repeat_offline_and_remove() {
    if unqualified_host() {
        return;
    }
    let f = Fixture::new(true);
    let bytes = tool("fixture resource");
    declare(&f, Some(&bytes));
    let u = f.upstream_with([(TOOL_ZIP.into(), bytes)]);
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(
        exports(&f),
        [
            "fixture-exe.exe",
            "fixture.cmd",
            "node.exe",
            "pnpm.cmd",
            "pnpx.cmd"
        ]
    );
    let path = f.plan().path(Some(system32().as_os_str())).unwrap();
    // The `.cmd` forwarder runs the real launcher, so `%~dp0` is its tree.
    assert_eq!(exit_code(&path, "fixture"), Some(5));
    // PATHEXT lookup is case-insensitive and reaches the `.exe` hard link.
    assert_eq!(exit_code(&path, "FIXTURE-EXE /d /c exit 9"), Some(9));

    // Frozen, check, and offline repeats neither download nor republish.
    let (manifest, hits) = (f.manifest(), u.hits());
    for flags in [FROZEN, CHECK, OFFLINE] {
        assert_eq!(f.run(&u, flags).unwrap(), 0, "{flags:?}");
    }
    assert_eq!((f.manifest(), u.hits()), (manifest, hits));

    // Removing the declaration removes its lock entry and exports.
    declare(&f, None);
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(exports(&f), ["node.exe", "pnpm.cmd", "pnpx.cmd"]);
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    assert!(lock["tools"].get("fixture").is_none());
    assert_eq!(u.hits(), hits);
    f.no_execution();
}

#[test]
fn a_running_managed_executable_never_blocks_rollback_or_upgrade() {
    if unqualified_host() {
        return;
    }
    let f = Fixture::new(true);
    let first = tool("fixture resource");
    declare(&f, Some(&first));
    let u = f.upstream_with([(TOOL_ZIP.into(), first.clone())]);
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    // The selected executable stays open in a running process. A builtin
    // `pause` on a held stdin pipe blocks without a child process to leak.
    let bin = f.plan().path_prepend().to_path_buf();
    let mut running = Command::new(bin.join("fixture-exe.exe"))
        .args(["/d", "/c", "pause"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // An upgrade whose bytes fail verification keeps the selection.
    let second = tool("upgraded resource");
    declare(&f, Some(&second));
    let manifest = f.manifest();
    assert!(f.run(&u, LOCAL).is_err());
    assert_eq!(f.manifest(), manifest);
    // turbo.lock already names the new digest, so run and exec would ask for
    // setup, but the selected generation still runs intact.
    let previous = std::env::join_paths([bin.clone(), system32()]).unwrap();
    assert_eq!(exit_code(&previous, "fixture"), Some(5));

    // A verified upgrade publishes a new generation beside the locked one.
    let v = f.upstream_with([(TOOL_ZIP.into(), second)]);
    assert_eq!(f.run(&v, LOCAL).unwrap(), 0);
    assert_ne!(f.manifest(), manifest);
    let path = f.plan().path(Some(system32().as_os_str())).unwrap();
    assert_eq!(exit_code(&path, "fixture"), Some(1));
    assert!(running.try_wait().unwrap().is_none());
    running.kill().unwrap();
    running.wait().unwrap();
    assert_eq!(f.run(&v, CHECK).unwrap(), 0);
    f.no_execution();
}
