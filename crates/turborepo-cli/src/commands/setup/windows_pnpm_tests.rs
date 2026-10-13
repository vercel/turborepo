//! Native Windows x64 pnpm 11 (ES module entry points on the managed node.exe)
//! and pnpm 12 (the verified `@pnpm/exe.win32-x64` binary beside its wrapper's
//! `dist/`), through portable lock creation and reuse. Local fixtures only.
use turborepo_setup::activation::ActivationPlan;

use super::*;

const NAMES: [&str; 4] = ["pn", "pnpm", "pnpx", "pnx"];
const NATIVES: [(&str, &str); 6] = [
    ("darwin-x64", "pnpm"),
    ("darwin-arm64", "pnpm"),
    ("linux-x64", "pnpm"),
    ("linux-arm64", "pnpm"),
    ("win32-x64", "pnpm.exe"),
    ("win32-arm64", "pnpm.exe"),
];

/// Metadata and tarball routes for one exact registry package.
fn package(name: &str, version: &str, bytes: Vec<u8>) -> [(String, Vec<u8>); 2] {
    let base = name.rsplit('/').next().unwrap();
    let tarball = format!("/{name}/-/{base}-{version}.tgz");
    let metadata = json!({"name":name,"version":version,"dist":{
        "tarball":format!("https://registry.npmjs.org{tarball}"),"integrity":integrity(&bytes)}});
    [
        (
            format!("/{name}/{version}"),
            metadata.to_string().into_bytes(),
        ),
        (tarball, bytes),
    ]
}

fn declare(f: &Fixture, version: &str) {
    let manifest = json!({ "packageManager": format!("pnpm@{version}") });
    fs::write(f.owned.root().join("package.json"), manifest.to_string()).unwrap();
}

/// Local setup creates the lock, then frozen, local and check repeats make no
/// requests. Returns the activation and its sorted bin exports.
fn provision(f: &Fixture, routes: Vec<(String, Vec<u8>)>) -> (ActivationPlan, Vec<String>) {
    let u = f.upstream_with(routes);
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let (hits, manifest) = (u.hits(), f.manifest());
    for flags in [FROZEN, LOCAL, CHECK] {
        assert_eq!(f.run(&u, flags).unwrap(), 0);
    }
    assert_eq!(u.hits(), hits);
    assert_eq!(f.manifest(), manifest);
    let plan = f.plan();
    let mut names: Vec<_> = fs::read_dir(plan.path_prepend().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    (plan, names)
}

fn launcher(plan: &ActivationPlan, name: &str) -> String {
    let pnpm = plan.tools().iter().find(|tool| tool.id == "pnpm").unwrap();
    let path = &pnpm.executables[name];
    assert!(path.starts_with("managed-node-"), "{path}");
    assert!(path.ends_with(&format!("/{name}.cmd")), "{path}");
    let tools = plan.path_prepend().unwrap().join("../tools/pnpm");
    fs::read_to_string(tools.join(path)).unwrap()
}

const EXPORTS: [&str; 5] = ["node.exe", "pn.cmd", "pnpm.cmd", "pnpx.cmd", "pnx.cmd"];

#[test]
fn pnpm_11_runs_its_module_entry_points_on_the_managed_node() {
    if unqualified_host() {
        return;
    }
    let version = "11.28.2";
    let manifest = json!({"name":"pnpm","version":version,"bin":{"pnpm":"bin/pnpm.mjs",
        "pnpx":"bin/pnpx.mjs","pn":"bin/pnpm.mjs","pnx":"bin/pnpx.mjs"}});
    let bytes = tgz(&[
        ("package.json", &manifest.to_string()),
        ("bin/pnpm.mjs", "throw new Error('must not run');"),
        ("bin/pnpx.mjs", "throw new Error('must not run');"),
        // Only a Corepack shim since pnpm 11; never an entry point.
        ("bin/pnpm.cjs", "throw new Error('must not run');"),
    ]);
    let f = Fixture::new(false);
    declare(&f, version);
    let (plan, names) = provision(&f, package("pnpm", version, bytes).into());
    assert_eq!(names, EXPORTS);
    for (name, entry) in [
        ("pnpm", "pnpm"),
        ("pn", "pnpm"),
        ("pnpx", "pnpx"),
        ("pnx", "pnpx"),
    ] {
        assert_eq!(
            launcher(&plan, name),
            format!("@\"%~dp0..\\..\\node\\node.exe\" \"%~dp0..\\bin\\{entry}.mjs\" %*\r\n")
        );
    }
    let path = plan.path(Some(system32().as_os_str())).unwrap();
    for (code, name) in NAMES.iter().enumerate() {
        let line = format!("{name} /d /c exit {}", code + 10);
        assert_eq!(exit_code(&path, &line), Some(code as i32 + 10), "{name}");
    }
    f.no_execution();
}

#[test]
fn pnpm_12_locks_every_platform_and_runs_the_windows_binary_beside_its_wrapper() {
    if unqualified_host() {
        return;
    }
    let version = "12.11.2";
    let bin: serde_json::Map<_, _> = NAMES.iter().map(|n| (n.to_string(), json!(n))).collect();
    let wrapper = json!({"name":"pnpm","version":version,"bin":bin,
        "scripts":{"preinstall":"node install.js","postinstall":"node install.js"}});
    let placeholder = "echo placeholder >&2\nexit 97\n";
    let mut files = vec![
        ("package.json", wrapper.to_string()),
        ("install.js", "process.exit(99)".into()),
        ("bin/pnpm.mjs", "process.exit(99)".into()),
        ("dist/node-gyp-bin/node-gyp.cmd", "@exit /b 99\r\n".into()),
    ];
    files.extend(NAMES.map(|name| (name, placeholder.to_string())));
    let files: Vec<_> = files.iter().map(|(p, b)| (*p, b.as_str())).collect();
    let mut routes: Vec<_> = package("pnpm", version, tgz(&files)).into();
    // A real PE image stands in for the Windows binary so launchers run it.
    let windows = fs::read(system32().join("cmd.exe")).unwrap();
    for (target, binary) in NATIVES {
        let name = format!("@pnpm/exe.{target}");
        let manifest = json!({"name":name,"version":version}).to_string();
        let mut tar = tar::Builder::new(Vec::new());
        let data = if target == "win32-x64" {
            windows.as_slice()
        } else {
            b"native".as_slice()
        };
        for (path, data) in [("package.json", manifest.as_bytes()), (binary, data)] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, format!("package/{path}"), data)
                .unwrap();
        }
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&tar.into_inner().unwrap()).unwrap();
        routes.extend(package(&name, version, gzip.finish().unwrap()));
    }
    let f = Fixture::new(false);
    declare(&f, version);
    let (plan, names) = provision(&f, routes);
    assert_eq!(names, EXPORTS);

    // Every platform is locked, so a new host never mutates the lock.
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    let artifacts = lock["tools"]["pnpm"]["installation"]["artifacts"]
        .as_object()
        .unwrap();
    assert_eq!(artifacts.len(), 6);
    let native = &artifacts["windows-x64"]["native"];
    assert_eq!(
        native["url"],
        format!("https://registry.npmjs.org/@pnpm/exe.win32-x64/-/exe.win32-x64-{version}.tgz")
    );
    let mapped: Value = NAMES
        .iter()
        .map(|n| (n.to_string(), json!("pnpm.exe")))
        .collect();
    assert_eq!(native["executables"], mapped);

    // The binary sits at the wrapper's own Windows Corepack location, beside
    // `dist/`, and launchers run it by that path, never a bin hard link.
    let tools = plan.path_prepend().unwrap().join("../tools/pnpm");
    assert_eq!(fs::read(tools.join("pnpm-native.exe")).unwrap(), windows);
    assert!(tools.join("dist/node-gyp-bin/node-gyp.cmd").is_file());
    assert!(!tools.join("pnpm-native").exists());
    for name in NAMES {
        let dlx = if matches!(name, "pnpx" | "pnx") {
            " dlx"
        } else {
            ""
        };
        assert_eq!(
            launcher(&plan, name),
            format!("@\"%~dp0..\\pnpm-native.exe\"{dlx} %*\r\n")
        );
    }
    let path = plan.path(Some(system32().as_os_str())).unwrap();
    for (code, name) in NAMES.iter().enumerate() {
        let line = format!("{name} /d /c exit {}", code + 20);
        assert_eq!(exit_code(&path, &line), Some(code as i32 + 20), "{name}");
    }
    f.no_execution();
}
