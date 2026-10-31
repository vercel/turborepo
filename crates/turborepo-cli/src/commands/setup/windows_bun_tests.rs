//! Native Windows x64 Bun: the official x64 baseline zip, `bun.exe` and
//! `bunx.exe` hard-link exports, Bun-only repositories without Node, reuse,
//! and fail-closed locks. Local fixtures only.
use super::*;

const VERSION: &str = "1.2.23";
const RELEASE: &str = "/oven-sh/bun/releases/download/bun-v1.2.23";

/// Made on Unix with a root directory entry, like the official archive. A
/// real PE image stands in for bun.exe so the exports run.
fn archive() -> Vec<u8> {
    let bun = fs::read(system32().join("cmd.exe")).unwrap();
    zip_in("bun-windows-x64-baseline", true, &[("bun.exe", bun)])
}

/// The release checksums list every target; only the x64 baseline is served.
fn routes(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    let digest = Sha256::digest(archive);
    let mut sums = format!("{digest:x}  bun-windows-x64-baseline.zip\n");
    for other in [
        "darwin-x64-baseline",
        "darwin-aarch64",
        "linux-x64-baseline",
        "linux-aarch64",
        "windows-x64",
        "windows-aarch64",
    ] {
        let digest = Sha256::digest(other);
        sums.push_str(&format!("{digest:x}  bun-{other}.zip\n"));
    }
    vec![
        (format!("{RELEASE}/SHASUMS256.txt"), sums.into_bytes()),
        (
            format!("{RELEASE}/bun-windows-x64-baseline.zip"),
            archive.to_vec(),
        ),
    ]
}

/// A repository that declares Bun, with or without a Node declaration.
fn bun_fixture(node: bool) -> Fixture {
    let f = Fixture::new(false);
    let root = f.owned.root();
    if !node {
        fs::remove_file(root.join(".nvmrc")).unwrap();
    }
    let manifest = json!({ "packageManager": format!("bun@{VERSION}") });
    fs::write(root.join("package.json"), manifest.to_string()).unwrap();
    f
}

fn lock(f: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap()
}

#[test]
fn bun_with_and_without_node_installs_the_baseline_build_and_repeats_without_traffic() {
    if unqualified_host() {
        return;
    }
    for node in [false, true] {
        let f = bun_fixture(node);
        let u = f.upstream_with(routes(&archive()));
        assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
        let lock = lock(&f);
        // x64 locks Bun's baseline build, as on Unix. Windows arm64 is locked
        // for portability but never installed by an x64 host.
        let artifacts = &lock["tools"]["bun"]["installation"]["artifacts"];
        let windows = &artifacts["windows-x64"]["distribution"];
        assert_eq!(
            windows["url"],
            format!("https://github.com{RELEASE}/bun-windows-x64-baseline.zip")
        );
        assert_eq!(windows["rootPrefix"], "bun-windows-x64-baseline");
        assert_eq!(
            windows["executables"],
            json!({"bun":"bun.exe","bunx":"bun.exe"})
        );
        assert!(artifacts["windows-arm64"].is_object());
        assert_eq!(lock["tools"].get("node").is_some(), node);
        let downloads: Vec<_> = u
            .requests()
            .into_iter()
            .filter(|path| path.ends_with(".zip") && path.contains("/bun-"))
            .collect();
        assert_eq!(
            downloads,
            [format!("{RELEASE}/bun-windows-x64-baseline.zip")]
        );

        let activation = f.activation();
        let bin = activation.bin().to_path_buf();
        let mut names: Vec<_> = fs::read_dir(&bin)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let expected: &[&str] = if node {
            // With Node and its bundled npm, as on Unix.
            &["bun.exe", "bunx.exe", "node.exe", "npm.cmd", "npx.cmd"]
        } else {
            &["bun.exe", "bunx.exe"]
        };
        assert_eq!(names, expected);
        // Both names are the verified binary, which Bun dispatches on by name.
        let binary = f.installed().trees["bun"].join("bun.exe");
        for name in ["bun.exe", "bunx.exe"] {
            assert!(same_file::is_same_file(bin.join(name), &binary).unwrap());
        }
        assert!(activation.vars().is_empty());
        let path = activation.path(Some(system32().as_os_str())).unwrap();
        assert_eq!(exit_code(&path, "bun /d /c exit 4"), Some(4));
        assert_eq!(exit_code(&path, "bunx /d /c exit 5"), Some(5));

        let (hits, manifest) = (u.hits(), f.manifest());
        for flags in [FROZEN, LOCAL, CHECK] {
            assert_eq!(f.run(&u, flags).unwrap(), 0);
        }
        assert_eq!(u.hits(), hits);
        assert_eq!(f.manifest(), manifest);
        f.no_execution();
    }
}

#[test]
fn bun_locks_without_the_exact_windows_x64_baseline_fail_before_installation() {
    if unqualified_host() {
        return;
    }
    let f = bun_fixture(false);
    let u = f.upstream_with(routes(&archive()));
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let (locked, manifest) = (lock(&f), f.manifest());
    let pointer = "/tools/bun/installation/artifacts";
    let mut missing = locked.clone();
    missing
        .pointer_mut(pointer)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("windows-x64");
    let mut standard = locked.clone();
    *standard
        .pointer_mut(&format!("{pointer}/windows-x64/distribution/url"))
        .unwrap() = json!(format!("https://github.com{RELEASE}/bun-windows-x64.zip"));
    for edited in [missing, standard] {
        fs::write(f.owned.root().join("turbo.lock"), edited.to_string()).unwrap();
        let hits = u.hits();
        assert!(f.run(&u, FROZEN).is_err(), "{edited}");
        assert!(f.run(&u, CHECK).is_err(), "{edited}");
        assert_eq!(u.hits(), hits);
        assert_eq!(f.manifest(), manifest);
    }
    f.no_execution();
}
