use std::collections::BTreeSet;

use super::*;
use crate::{
    bundled_npm,
    execution_identity::{ExecutionContext, ExecutionSnapshot, Libc, SelectedInstallation},
    lock::{Document, Snapshot, WriteOutcome, reconcile},
    node_resolution::{ChecksumManifest, resolve},
};

const TARGET: Platform = Platform::LinuxX64Gnu;
const PACKAGE: &str =
    r#"{"name":"npm","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#;

fn capture(root: &Path, manifest: Value) -> Snapshot {
    fs::write(root.join(".nvmrc"), "24.0.0").unwrap();
    fs::write(
        root.join("package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    Snapshot::capture(root).unwrap()
}
fn node(snapshot: &Snapshot, bytes: &[u8]) -> lock::Tool {
    let index = serde_json::to_vec(
        &json!([{"version":"v24.0.0","npm":"11.6.1","lts":false,"files":["linux-x64"]}]),
    )
    .unwrap();
    let checksums = format!(
        "{:x}  node-v24.0.0-linux-x64.tar.gz\n",
        Sha256::digest(bytes)
    );
    resolve(
        &snapshot.node_requirements().unwrap(),
        &index,
        ChecksumManifest {
            version: "24.0.0",
            bytes: checksums.as_bytes(),
        },
        true,
    )
    .unwrap()
    .into_tool()
}
fn candidate(snapshot: &Snapshot, node: lock::Tool) -> Lock {
    let npm = bundled_npm::resolve(snapshot, &node).unwrap();
    Lock::new(Document {
        schema_version: 0,
        tools: [("node".into(), node), ("npm".into(), npm)].into(),
    })
    .unwrap()
}
fn execution(
    lock: &Lock,
    ids: &[&str],
) -> Result<ExecutionSnapshot, crate::execution_identity::Error> {
    let context = ExecutionContext::new(
        turborepo_platform::Platform::new(
            turborepo_platform::OperatingSystem::Linux,
            turborepo_platform::Architecture::X64,
        ),
        Libc::Gnu {
            abi: "glibc-2.31".into(),
        },
        "x86_64-unknown-linux-gnu".into(),
        BTreeSet::from(["x86-64-v2".into()]),
    )
    .unwrap();
    ExecutionSnapshot::select(lock, context, &ids.iter().map(|id| (*id).into()).collect())
}
fn archive(package: &str) -> Vec<u8> {
    unix_archive_with_package("linux", "x64", false, false, package)
}

#[tokio::test]
async fn bundled_native_resolution_publication_execution_and_one_owner_generation() {
    for manifest in [
        json!({"packageManager":"npm@11.6.1"}),
        json!({"devEngines":{"packageManager":{"name":"npm","version":"11.6.1"}}}),
    ] {
        let repo = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(repo.path())
                .status()
                .unwrap()
                .success()
        );
        fs::write(repo.path().join(".gitignore"), "/.turbo/\n").unwrap();
        let captured = capture(repo.path(), manifest);
        let bytes = archive(PACKAGE);
        let lock = candidate(&captured, node(&captured, &bytes));
        let outcome = reconcile::reconcile(&captured, reconcile::Mode::Local, false, |_| {
            Ok(lock.clone())
        })
        .unwrap();
        assert_eq!(outcome.publication, Some(WriteOutcome::Written));
        let committed = Lock::read(repo.path()).unwrap().unwrap();
        assert!(committed.matches_native(captured.declarations()).unwrap());
        assert_eq!(
            committed.tools()["npm"].declarations,
            captured.declarations()["npm"]
        );
        assert_eq!(committed.tools()["npm"].version, "11.6.1");
        assert!(
            matches!(&committed.tools()["npm"].installation, Installation::Bundled { owner } if owner == "node")
        );
        let before = fs::read(repo.path().join("turbo.lock")).unwrap();
        let frozen = Snapshot::capture(repo.path()).unwrap();
        reconcile::reconcile(&frozen, reconcile::Mode::Frozen, true, |_| {
            panic!("no resolution")
        })
        .unwrap();

        let snapshot = execution(&committed, &["node", "npm"]).unwrap();
        assert_eq!(snapshot.tools()["npm"].version, "11.6.1");
        assert!(
            matches!(&snapshot.tools()["npm"].installation, SelectedInstallation::Bundled { owner } if owner == "node")
        );
        assert!(matches!(
            execution(&committed, &["npm"]),
            Err(crate::execution_identity::Error::MissingOwner)
        ));
        assert!(matches!(
            execution(&committed, &["node"]),
            Err(crate::execution_identity::Error::MissingOwner)
        ));
        let mut undeclared = committed.document().clone();
        undeclared.tools.remove("npm");
        assert_ne!(
            snapshot.fingerprint(),
            execution(&Lock::new(undeclared).unwrap(), &["node"])
                .unwrap()
                .fingerprint()
        );
        let mut changed = committed.document().clone();
        changed.tools.get_mut("npm").unwrap().version = "11.6.2".into();
        changed
            .tools
            .get_mut("node")
            .unwrap()
            .options
            .insert("bundled-npm".into(), vec!["11.6.2".into()]);
        assert_ne!(
            snapshot.fingerprint(),
            execution(&Lock::new(changed).unwrap(), &["node", "npm"])
                .unwrap()
                .fingerprint()
        );
        let mut changed = committed.document().clone();
        let Installation::Managed { artifacts } =
            &mut changed.tools.get_mut("node").unwrap().installation
        else {
            panic!()
        };
        artifacts
            .get_mut(&TARGET)
            .unwrap()
            .get_mut("distribution")
            .unwrap()
            .sha256 = "a".repeat(64);
        assert_ne!(
            snapshot.fingerprint(),
            execution(&Lock::new(changed).unwrap(), &["node", "npm"])
                .unwrap()
                .fingerprint()
        );

        let plan = NodePlan::from_lock(&committed, TARGET).unwrap();
        let upstream = serve(bytes).await;
        let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
        let mut store = Store::open(repo.path()).unwrap();
        assert_eq!(
            install(&mut store, &plan, &transport).await.unwrap(),
            Outcome::Replaced
        );
        assert_eq!(
            store.current().unwrap().unwrap().tools,
            vec![plan.inventory_tool().clone()]
        );
        recovery::assert_scoped_resource(&store);
        assert_eq!(
            install(&mut store, &plan, &transport).await.unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(upstream.requests.load(Ordering::SeqCst), 1);
        // The loopback server only accepts Node distribution requests; no registry
        // transport exists anywhere in this path, including a network-free repeat.
        upstream.task.abort();
        let mut sibling = plan.inventory_tool().clone();
        sibling.id = "fixture".into();
        sibling.executables = [("fixture".into(), "bin/fixture".into())].into();
        let desired = [plan.inventory_tool().clone(), sibling];
        assert!(
            plan.prepare_if_needed(&store, &desired, &transport)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .reconcile(&desired, |tool, path| {
                    assert_eq!(tool.id, "fixture");
                    fs::create_dir(path.join("bin"))?;
                    use std::os::unix::fs::PermissionsExt;
                    let executable = path.join("bin/fixture");
                    fs::write(&executable, "#!/bin/sh\nexit 0\n")?;
                    fs::set_permissions(executable, fs::Permissions::from_mode(0o755))?;
                    Ok(())
                })
                .unwrap(),
            Outcome::Replaced
        );
        recovery::assert_scoped_resource(&store);
        assert!(store.is_current(&desired).unwrap());
        assert_eq!(fs::read(repo.path().join("turbo.lock")).unwrap(), before);
        let current = store.current().unwrap().unwrap();
        for name in ["npm", "npx"] {
            let output = std::process::Command::new(current.bin.join(name))
                .env_clear()
                .env("PATH", &current.bin)
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                format!("bundled-{name}")
            );
        }
    }
}

#[test]
fn native_nonmatching_ranges_and_unverified_registry_integrity_are_not_bundled() {
    for manifest in [
        json!({"packageManager":"npm@11.6.2"}),
        json!({"packageManager":"npm@11.6.1+vendor.1"}),
        json!({"devEngines":{"packageManager":{"name":"npm","version":"11.x"}}}),
        json!({"packageManager":format!("npm@11.6.1+sha512.{}", "a".repeat(128))}),
        json!({"packageManager":"pnpm@11.6.1"}),
    ] {
        let repo = tempfile::tempdir().unwrap();
        let snapshot = capture(repo.path(), manifest);
        assert!(matches!(
            bundled_npm::resolve(&snapshot, &node(&snapshot, &archive(PACKAGE))),
            Err(bundled_npm::Error::NotMatching)
        ));
        assert!(!repo.path().join("turbo.lock").exists());
    }
}

#[test]
fn inconsistent_owners_identity_platforms_and_collisions_are_rejected() {
    let repo = tempfile::tempdir().unwrap();
    let captured = capture(repo.path(), json!({"packageManager":"npm@11.6.1"}));
    let good = candidate(&captured, node(&captured, &archive(PACKAGE)));
    let mut value = serde_json::to_value(good.document()).unwrap();
    value["tools"]["npm"]["options"] = json!({});
    for (pointer, replacement) in [
        ("/tools/npm/installation/owner", json!("absent")),
        ("/tools/npm/installation/owner", json!("npm")),
        ("/tools/npm/version", json!("11.6.2")),
        ("/tools/npm/options", json!({"invented":["identity"]})),
        ("/tools/node/options", json!({})),
        ("/tools/node/options", json!({"bundled-npm":["11.x"]})),
        (
            "/tools/node/installation",
            json!({"kind":"verify-system","executables":["node","npm","npx"]}),
        ),
        (
            "/tools/node/installation/artifacts/linux-x64-gnu/distribution/executables",
            json!({"node":"bin/node","npm":"bin/wrong","npx":"bin/npx"}),
        ),
    ] {
        let mut bad = value.clone();
        *bad.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            Lock::parse(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut bad = good.document().clone();
    let mut alias = bad.tools["npm"].clone();
    alias.adapter = "npm".into();
    bad.tools.insert("alias".into(), alias);
    assert!(Lock::new(bad).is_err());
    let mut bad = good.document().clone();
    let mut sibling = bad.tools["npm"].clone();
    sibling.installation = Installation::VerifySystem {
        executables: vec!["NPM".into()],
    };
    bad.tools.insert("fixture".into(), sibling);
    assert!(Lock::new(bad).is_err());
    let mut bad = good.document().clone();
    let Installation::Managed { artifacts } = &mut bad.tools.get_mut("node").unwrap().installation
    else {
        panic!()
    };
    let parts = artifacts.remove(&TARGET).unwrap();
    artifacts.insert(Platform::Any, parts);
    assert!(Lock::new(bad).is_err());
}

#[tokio::test]
async fn invalid_verified_packages_never_promote_and_legacy_inventory_cannot_assert_identity() {
    let repo = tempfile::tempdir().unwrap();
    let captured = capture(repo.path(), json!({"packageManager":"npm@11.6.1"}));
    let good_bytes = archive(PACKAGE);
    let good = candidate(&captured, node(&captured, &good_bytes));
    let good_plan = NodePlan::from_lock(&good, TARGET).unwrap();
    let upstream = serve(good_bytes).await;
    let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    install(&mut store, &good_plan, &transport).await.unwrap();
    let manifest = repo.path().join(".turbo/tools/manifest.json");
    let before = fs::read(&manifest).unwrap();
    for package in [
        r#"{"name":"npm"}"#,
        r#"{"name":"npm","version":"11.6.2","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#,
        r#"{"name":"other","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#,
        r#"{"name":"npm","version":"11.6.1","version":"11.6.2","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js"}}"#,
        r#"{"name":"npm","version":"11.6.1","bin":{"npm":"bin/npx-cli.js","npx":"bin/npx-cli.js"}}"#,
        r#"{"name":"npm","version":"11.6.1","bin":{"npm":"bin/npm-cli.js","npx":"bin/npx-cli.js","extra":"bin/npm-cli.js"}}"#,
    ] {
        let bytes = archive(package);
        let lock = candidate(&captured, node(&captured, &bytes));
        let plan = NodePlan::from_lock(&lock, TARGET).unwrap();
        let upstream = serve(bytes).await;
        let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
        assert!(matches!(
            install(&mut store, &plan, &transport).await,
            Err(Error::InvalidBundledNpm)
        ));
        assert_eq!(fs::read(&manifest).unwrap(), before);
        // A legacy inventory can have this SAME Node artifact/mappings with no
        // semantic npm identity. Reuse must verify resources, not trust inventory.
        let mut legacy = lock.document().clone();
        legacy.tools.remove("npm");
        legacy.tools.get_mut("node").unwrap().options.clear();
        let legacy = NodePlan::from_lock(&Lock::new(legacy).unwrap(), TARGET).unwrap();
        let empty = tempfile::tempdir().unwrap();
        let mut legacy_store = Store::open(empty.path()).unwrap();
        install(&mut legacy_store, &legacy, &transport)
            .await
            .unwrap();
        let installed = fs::read(empty.path().join(".turbo/tools/manifest.json")).unwrap();
        assert!(matches!(
            install(&mut legacy_store, &plan, &transport).await,
            Err(Error::InvalidBundledNpm)
        ));
        assert_eq!(
            fs::read(empty.path().join(".turbo/tools/manifest.json")).unwrap(),
            installed
        );
        assert_eq!(upstream.requests.load(Ordering::SeqCst), 2);
    }
    let mut collision = good_plan.inventory_tool().clone();
    collision.id = "npm".into();
    assert!(
        good_plan
            .prepare_if_needed(
                &store,
                &[good_plan.inventory_tool().clone(), collision],
                &transport
            )
            .await
            .is_err()
    );
    assert_eq!(upstream.requests.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(manifest).unwrap(), before);
}
