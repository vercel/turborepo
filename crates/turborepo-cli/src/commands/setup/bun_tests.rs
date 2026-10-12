//! Locked Bun provisioning through `turbo setup`, without a global Bun.
use turborepo_setup::{lock::Platform, test_support::stored_zip};

use super::*;

const RELEASE: &str = "/oven-sh/bun/releases/download/bun-v1.2.23";

#[test]
fn locked_bun_provisions_checks_repeats_and_rejects_frozen_drift() {
    let mut f = Fixture::new();
    let platform = provision::platform().unwrap();
    let target = match platform {
        Platform::MacosArm64 => "darwin-aarch64",
        Platform::MacosX64 => "darwin-x64-baseline",
        Platform::LinuxX64Gnu => "linux-x64-baseline",
        _ => "linux-aarch64",
    };
    // Exits 93 when run: proves the shims execute this verified binary.
    let archive = stored_zip(&format!("bun-{target}/bun"), b"#!/bin/sh\nexit 93\n");
    let path = format!("{RELEASE}/bun-{target}.zip");
    let key = serde_json::to_value(platform).unwrap();
    let key = key.as_str().unwrap();
    let tools = f.lock["tools"].as_object_mut().unwrap();
    tools.remove("pnpm");
    tools.insert(
        "bun".into(),
        json!({"adapter":"bun","version":"1.2.23","declarations":[],
            "installation":{"kind":"managed","artifacts":{key:{"distribution":{
                "url":format!("https://github.com{path}"),"sha256":format!("{:x}", Sha256::digest(&archive)),
                "format":"zip","rootPrefix":format!("bun-{target}"),"executables":{"bun":"bun","bunx":"bun"}
            }}}}}),
    );
    f.declare(json!({"packageManager":"bun@1.2.23"}));
    let lock_path = f.owned.root().join("turbo.lock");
    let canonical = turborepo_setup::lock::Lock::parse(&fs::read(&lock_path).unwrap()).unwrap();
    fs::write(&lock_path, canonical.canonical_bytes().unwrap()).unwrap();
    let lock = fs::read(&lock_path).unwrap();
    let u = LoopbackServer::new(
        [(f.node_path.clone(), f.node.clone()), (path, archive)],
        |_| {},
    )
    .unwrap();

    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 2);
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    let selected: Vec<_> = current.tools.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(selected, ["bun", "node"]);
    let scratch = tempfile::tempdir().unwrap();
    for name in ["bun", "bunx"] {
        let output = std::process::Command::new(current.bin.join(name))
            .env("PATH", scratch.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(93));
    }
    drop(store);
    let manifest = f.manifest().unwrap();
    for flags in [
        FROZEN,
        &["--tools-only", "--no-frozen"],
        &["--check", "--tools-only"],
    ] {
        assert_eq!(f.run(&u, flags).unwrap(), 0);
    }
    fs::write(
        f.owned.root().join("package.json"),
        json!({"packageManager":"bun@1.2.24"}).to_string(),
    )
    .unwrap();
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 2);
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(fs::read(&lock_path).unwrap(), lock);
    f.no_execution();
}
