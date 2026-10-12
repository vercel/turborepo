//! Locked Yarn classic and modern provisioning through `turbo setup`.
use sha2::Sha512;

use super::{first_lock::base64, *};

fn yarn(version: &str, package: &str) -> (Fixture, LoopbackServer) {
    let mut f = Fixture::new();
    let base = package.rsplit('/').next().unwrap();
    let tarball = format!("/{package}/-/{base}-{version}.tgz");
    let manifest = json!({"name":package,"version":version,
        "bin":{"yarn":"./bin/yarn.js","yarnpkg":"./bin/yarn.js"},
        "scripts":{"preinstall":"touch hook-ran"}});
    let bytes = archive(
        "package",
        &[
            ("package.json", &manifest.to_string()),
            ("bin/yarn.js", &format!("// yarn {version}\n")),
        ],
    );
    let tools = f.lock["tools"].as_object_mut().unwrap();
    tools.remove("pnpm");
    tools.insert(
        "yarn".into(),
        json!({"adapter":"yarn","version":version,"declarations":[],
        "installation":{"kind":"managed","artifacts":{"any":{"package":{
            "url":format!("https://registry.npmjs.org{tarball}"),
            "sha256":format!("{:x}", Sha256::digest(&bytes)),"format":"tar-gz",
            "rootPrefix":"package","executables":{"yarn":"bin/yarn.js","yarnpkg":"bin/yarn.js"}
        }}}}}),
    );
    f.declare(json!({ "packageManager": format!("yarn@{version}") }));
    let path = f.owned.root().join("turbo.lock");
    let canonical = turborepo_setup::lock::Lock::parse(&fs::read(&path).unwrap()).unwrap();
    fs::write(path, canonical.canonical_bytes().unwrap()).unwrap();
    let metadata = json!({"name":package,"version":version,"dist":{
        "tarball":format!("https://registry.npmjs.org{tarball}"),
        "integrity":format!("sha512-{}", base64(&Sha512::digest(&bytes)))
    }});
    let upstream = LoopbackServer::new(
        [
            (f.node_path.clone(), f.node.clone()),
            (
                format!("/{package}/{version}"),
                metadata.to_string().into_bytes(),
            ),
            (tarball, bytes),
        ],
        |_| {},
    )
    .unwrap();
    (f, upstream)
}

/// Frozen provisioning, shims, local and check repeats, then frozen drift.
fn provisions_checks_repeats_and_rejects_drift(version: &str, package: &str, drifted: &str) {
    let (f, u) = yarn(version, package);
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    let selected: Vec<_> = current
        .tools
        .iter()
        .map(|tool| (tool.id.as_str(), tool.version.as_str()))
        .collect();
    assert_eq!(selected, [("node", "24.0.0"), ("yarn", version)]);
    let scratch = tempfile::tempdir().unwrap();
    for name in ["yarn", "yarnpkg"] {
        // The fixture's managed Node exits 91: shims never use a host Node.
        let output = std::process::Command::new(current.bin.join(name))
            .env("PATH", scratch.path())
            .current_dir(scratch.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(91));
    }
    drop(store);
    let manifest = f.manifest().unwrap();
    for flags in [
        FROZEN,
        &["--tools-only", "--no-frozen"],
        &["--check", "--tools-only"],
        &["--plan", "--tools-only"],
    ] {
        assert_eq!(f.run(&u, flags).unwrap(), 0);
    }
    assert_eq!(u.hits(), 3);
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    fs::write(
        f.owned.root().join("package.json"),
        json!({ "packageManager": drifted }).to_string(),
    )
    .unwrap();
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 3);
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    f.no_execution();
}

#[test]
fn locked_yarn_classic_provisions_checks_repeats_and_rejects_frozen_drift() {
    provisions_checks_repeats_and_rejects_drift("1.22.22", "yarn", "yarn@1.22.21");
}

#[test]
fn yarn_without_node_rejects_before_writers_even_with_native_uv() {
    let (f, u) = yarn("1.22.22", "yarn");
    let root = f.owned.root();
    fs::remove_file(root.join(".nvmrc")).unwrap();
    fs::remove_file(root.join("turbo.lock")).unwrap();
    fs::write(
        root.join("turbo.json"),
        r#"{"futureFlags":{"experimentalSetup":true,"experimentalPythonWorkspaces":true}}"#,
    )
    .unwrap();
    fs::write(root.join("uv.toml"), "required-version = '>=0.9.0'\n").unwrap();
    let before: Vec<_> = ["package.json", "turbo.json", "uv.toml", ".gitignore"]
        .map(|name| (name, fs::read(root.join(name)).unwrap()))
        .into();
    let error = f.run(&u, &["--tools-only", "--no-frozen"]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("a native Node declaration is required"),
        "{error}"
    );
    assert_eq!(u.hits(), 0);
    assert!(!root.join(".turbo").exists());
    assert!(!root.join("turbo.lock").exists());
    for (name, bytes) in before {
        assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
    }
    f.no_execution();
}

#[test]
fn locked_modern_yarn_provisions_checks_repeats_and_rejects_frozen_drift() {
    provisions_checks_repeats_and_rejects_drift("4.5.0", "@yarnpkg/cli-dist", "yarn@4.6.0");
}
