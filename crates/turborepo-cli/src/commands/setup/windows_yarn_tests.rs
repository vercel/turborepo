//! Native Windows x64 Yarn classic and modern Yarn: portable lock creation,
//! `.cmd` launchers that run the managed node.exe, reuse, Corepack bundle
//! pins, and a committed `yarnPath`. Local fixtures only.
use super::*;

const CLASSIC: (&str, &str) = ("1.22.22", "yarn");
const MODERN: (&str, &str) = ("4.5.0", "@yarnpkg/cli-dist");
// Never run: the fixture node.exe is cmd.exe, which ignores this path.
const BUNDLE: &str = "throw new Error('must not run');";

/// Registry routes for one release: exact metadata and its tarball.
fn routes(version: &str, package: &str) -> Vec<(String, Vec<u8>)> {
    let base = package.rsplit('/').next().unwrap();
    let tarball = format!("/{package}/-/{base}-{version}.tgz");
    let manifest = json!({"name":package,"version":version,
        "bin":{"yarn":"./bin/yarn.js","yarnpkg":"./bin/yarn.js"},
        "scripts":{"preinstall":"echo hook > hook-ran"}});
    let bytes = tgz(&[
        ("package.json", &manifest.to_string()),
        ("bin/yarn.js", BUNDLE),
        ("lib/resource", "yarn resource"),
    ]);
    let metadata = json!({"name":package,"version":version,"dist":{
        "tarball":format!("https://registry.npmjs.org{tarball}"),"integrity":integrity(&bytes)}});
    vec![
        (
            format!("/{package}/{version}"),
            metadata.to_string().into_bytes(),
        ),
        (tarball, bytes),
    ]
}

fn declare(f: &Fixture, manager: &str) {
    let manifest = json!({ "packageManager": manager }).to_string();
    fs::write(f.owned.root().join("package.json"), manifest).unwrap();
}

fn lock(f: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap()
}

#[test]
fn yarn_classic_and_modern_lock_launch_the_managed_node_and_repeat_without_traffic() {
    if unqualified_host() {
        return;
    }
    for ((version, package), drifted) in [(CLASSIC, "yarn@1.22.21"), (MODERN, "yarn@4.6.0")] {
        let f = Fixture::new(false);
        declare(&f, &format!("yarn@{version}"));
        let u = f.upstream_with(routes(version, package));
        assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
        // One portable registry package, as on Unix.
        let base = package.rsplit('/').next().unwrap();
        let yarn = &lock(&f)["tools"]["yarn"];
        assert_eq!(yarn["version"], version);
        let artifact = &yarn["installation"]["artifacts"]["any"]["package"];
        assert_eq!(
            artifact["url"],
            format!("https://registry.npmjs.org/{package}/-/{base}-{version}.tgz")
        );
        assert_eq!(
            artifact["executables"],
            json!({"yarn":"bin/yarn.js","yarnpkg":"bin/yarn.js"})
        );

        let activation = f.activation();
        let bin = activation.bin().to_path_buf();
        let mut names: Vec<_> = fs::read_dir(&bin)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        // Node's bundled npm is exported beside Yarn, as on Unix.
        assert_eq!(
            names,
            ["node.exe", "npm.cmd", "npx.cmd", "yarn.cmd", "yarnpkg.cmd"]
        );
        let installed = f.installed();
        let (node, yarn) = (&installed.trees["node"], &installed.trees["yarn"]);
        assert_eq!(
            fs::read(yarn.join("lib/resource")).unwrap(),
            b"yarn resource"
        );
        for name in ["yarn", "yarnpkg"] {
            assert_eq!(
                fs::read_to_string(bin.join(format!("{name}.cmd"))).unwrap(),
                format!(
                    "@\"{}\\node.exe\" \"{}\\bin\\yarn.js\" %*\r\n",
                    node.display(),
                    yarn.display()
                )
            );
        }
        // No runtime variables such as YARN_IGNORE_PATH, so Yarn keeps its
        // own committed-release handling.
        assert!(activation.vars().is_empty());
        let path = activation.path(Some(system32().as_os_str())).unwrap();
        assert_eq!(exit_code(&path, "yarn /d /c exit 6"), Some(6));
        assert_eq!(exit_code(&path, "yarnpkg /d /c exit 7"), Some(7));

        let (hits, manifest, locked) = (u.hits(), f.manifest(), lock(&f));
        for flags in [FROZEN, LOCAL, CHECK] {
            assert_eq!(f.run(&u, flags).unwrap(), 0);
        }
        assert_eq!(u.hits(), hits);
        assert_eq!(f.manifest(), manifest);
        // A changed declaration fails frozen setup before traffic or writes.
        declare(&f, drifted);
        assert!(f.run(&u, FROZEN).is_err());
        assert_eq!(u.hits(), hits);
        assert_eq!((f.manifest(), lock(&f)), (manifest, locked));
        f.no_execution();
    }
}

#[test]
fn modern_yarn_bundle_pins_verify_and_a_committed_yarn_path_is_left_to_yarn() {
    if unqualified_host() {
        return;
    }
    let (version, package) = MODERN;
    let bundle = format!("{:x}", Sha256::digest(BUNDLE));
    // Corepack pins modern Yarn's bin/yarn.js, never the registry archive.
    let archive = format!("{:x}", Sha256::digest(&routes(version, package)[1].1));
    let f = Fixture::new(false);
    declare(&f, &format!("yarn@{version}+sha256.{archive}"));
    let u = f.upstream_with(routes(version, package));
    assert!(f.run(&u, LOCAL).is_err());
    assert!(!f.owned.root().join("turbo.lock").exists());
    assert!(!f.trust().path().exists());

    // A committed release is Yarn's own redirect: setup never reads or runs it.
    let root = f.owned.root();
    fs::write(
        root.join(".yarnrc.yml"),
        "yarnPath: .yarn/releases/yarn-4.5.0.cjs\n",
    )
    .unwrap();
    fs::create_dir_all(root.join(".yarn/releases")).unwrap();
    let release = root.join(".yarn/releases/yarn-4.5.0.cjs");
    fs::write(&release, "throw new Error('committed release');").unwrap();
    declare(&f, &format!("yarn@{version}+sha256.{bundle}"));
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let installed = f.installed();
    let yarn = installed
        .tools
        .iter()
        .find(|tool| tool.id == "yarn")
        .unwrap();
    assert_eq!(yarn.version, version);
    // Yarn's bundle re-runs a committed yarnPath with its own process.execPath,
    // which the launcher makes the managed node.exe.
    let activation = f.activation();
    let launcher = fs::read_to_string(activation.bin().join("yarn.cmd")).unwrap();
    let node = installed.trees["node"].join("node.exe");
    assert!(launcher.starts_with(&format!("@\"{}\" ", node.display())));
    assert!(activation.vars().is_empty());
    assert_eq!(
        fs::read_to_string(&release).unwrap(),
        "throw new Error('committed release');"
    );
    let path = activation.path(Some(system32().as_os_str())).unwrap();
    assert_eq!(exit_code(&path, "yarn /d /c exit 5"), Some(5));
    f.no_execution();
}
