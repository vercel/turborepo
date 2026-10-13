//! Native Windows x64 Go through the real CLI path: the official zip layout,
//! `.exe` exports, a portable lock, and offline repeats. Local fixtures only.
use turborepo_setup::{go_resolution::INDEX_PATH, test_support::stored_zip_entries};

use super::*;

const GO: &str = r#"{"futureFlags":{"experimentalSetup":true,"experimentalGoWorkspaces":true}}"#;
const ZIP: &str = "/go/go1.22.12.windows-amd64.zip";

/// The official layout, including a UTF-8 named `go/test` fixture that setup
/// must never write. Real PE images stand in for the executables.
fn go_zip() -> Vec<u8> {
    let image = fs::read(system32().join("cmd.exe")).unwrap();
    stored_zip_entries(&[
        ("go/bin/go.exe", &image[..]),
        ("go/bin/gofmt.exe", &image[..]),
        ("go/src/go.mod", &b"module std\n"[..]),
        (
            "go/test/fixedbugs/issue27836.dir/\u{de}foo.go",
            &b"package p\n"[..],
        ),
    ])
}

/// The index lists every lockable target; only the windows-amd64 zip is served.
fn go_upstream(zip: &[u8]) -> LoopbackServer {
    let files: Vec<_> = ["darwin-amd64", "darwin-arm64", "linux-amd64", "linux-arm64"]
        .into_iter()
        .map(|target| (target, "tar.gz", "0".repeat(64)))
        .chain([
            ("windows-amd64", "zip", format!("{:x}", Sha256::digest(zip))),
            ("windows-arm64", "zip", "0".repeat(64)),
        ])
        .map(|(target, extension, sha256)| {
            let (os, arch) = target.split_once('-').unwrap();
            json!({"filename": format!("go1.22.12.{target}.{extension}"), "os": os,
                "arch": arch, "sha256": sha256, "version": "go1.22.12", "kind": "archive"})
        })
        .collect();
    let index = json!([{"version": "go1.22.12", "stable": true, "files": files}]);
    let routes = [
        (INDEX_PATH.into(), index.to_string().into_bytes()),
        (ZIP.into(), zip.to_vec()),
    ];
    LoopbackServer::new(routes, |_| {}).unwrap()
}

fn go_only() -> Fixture {
    let f = Fixture::new(false);
    let root = f.owned.root();
    fs::remove_file(root.join("package.json")).unwrap();
    fs::remove_file(root.join(".nvmrc")).unwrap();
    fs::write(root.join("turbo.json"), GO).unwrap();
    fs::write(root.join("go.work"), "go 1.22\n").unwrap();
    f
}

fn installed(f: &Fixture) -> turborepo_tool_install::Current {
    turborepo_tool_install::Store::inspect(f.owned.root())
        .unwrap()
        .unwrap()
}

#[test]
fn go_zip_installs_exe_exports_locks_every_target_and_repeats_offline() {
    if unqualified_host() {
        return;
    }
    let f = go_only();
    let u = go_upstream(&go_zip());
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.requests(), [INDEX_PATH, ZIP]);
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    let artifacts = &lock["tools"]["go"]["installation"]["artifacts"];
    let windows = &artifacts["windows-x64"]["distribution"];
    assert_eq!(windows["url"], format!("https://dl.google.com{ZIP}"));
    assert_eq!(windows["format"], "zip");
    assert_eq!(
        windows["executables"],
        json!({"go": "bin/go.exe", "gofmt": "bin/gofmt.exe"})
    );
    assert!(artifacts["linux-x64-gnu"].is_object() && artifacts["windows-arm64"].is_object());

    let current = installed(&f);
    let mut names: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["go.exe", "gofmt.exe"]);
    let tree = current.bin.join("../tools/go");
    for name in ["go.exe", "gofmt.exe"] {
        let target = tree.join("bin").join(name);
        assert!(same_file::is_same_file(current.bin.join(name), target).unwrap());
    }
    assert!(tree.join("src/go.mod").is_file() && !tree.join("test").exists());

    // Frozen, local and offline check repeats neither download nor republish.
    let manifest = f.manifest();
    let offline = LoopbackServer::new([], |_| {}).unwrap();
    for flags in [FROZEN, LOCAL, CHECK] {
        assert_eq!(f.run(&offline, flags).unwrap(), 0, "{flags:?}");
    }
    assert_eq!(offline.hits(), 0);
    assert_eq!(f.manifest(), manifest);
    f.no_execution();
}

#[test]
fn damaged_go_exports_fail_check_and_frozen_setup_repairs_them() {
    if unqualified_host() {
        return;
    }
    let f = go_only();
    let u = go_upstream(&go_zip());
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let manifest = f.manifest();
    fs::remove_file(installed(&f).bin.join("gofmt.exe")).unwrap();
    assert!(f.run(&u, CHECK).is_err());
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.requests(), [INDEX_PATH, ZIP, ZIP]);
    assert_ne!(f.manifest(), manifest);
    assert_eq!(f.run(&u, CHECK).unwrap(), 0);
    assert!(installed(&f).bin.join("gofmt.exe").is_file());
}
