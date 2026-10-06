use std::os::unix::fs::PermissionsExt;

use super::*;

struct ReadyNode {
    repo: tempfile::TempDir,
    store: Store,
    plan: NodePlan,
    upstream: Upstream,
    transport: NodeTransport,
}
async fn ready_node() -> ReadyNode {
    let bytes = unix_archive("darwin", "arm64", false, false);
    let upstream = serve(bytes.clone()).await;
    let transport = NodeTransport::loopback_http_for_tests(&upstream.origin).unwrap();
    let plan = plan(&fixture(Platform::MacosArm64, &bytes), Platform::MacosArm64).unwrap();
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    assert_eq!(
        install(&mut store, &plan, &transport).await.unwrap(),
        Outcome::Replaced
    );
    ReadyNode {
        repo,
        store,
        plan,
        upstream,
        transport,
    }
}
fn unavailable_transport() -> NodeTransport {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    NodeTransport::loopback_http_for_tests(&origin).unwrap()
}
fn sibling(node: &Tool) -> Tool {
    Tool {
        id: "fixture".into(),
        version: "1.0.0".into(),
        platform: node.platform.clone(),
        artifact_sha256: "b".repeat(64),
        executables: BTreeMap::from([("fixture".into(), "bin/fixture".into())]),
    }
}
fn stage_sibling(tool: &Tool, root: &Path) -> Result<(), turborepo_tool_install::Error> {
    assert_eq!(
        tool.id, "fixture",
        "healthy Node must be copied, not staged"
    );
    fs::create_dir(root.join("bin"))?;
    let path = root.join("bin/fixture");
    fs::write(&path, "#!/bin/sh\nexit 0\n")?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}
pub(super) fn assert_scoped_resource(store: &Store) {
    let current = store.current().unwrap().unwrap();
    let root = current.bin.parent().unwrap().join("tools/node");
    assert_eq!(
        fs::read(root.join(SCOPED_RESOURCE)).unwrap(),
        b"{\"name\":\"@npmcli/config\"}"
    );
}
fn assert_no_generation(repo: &Path) {
    assert!(
        fs::read_dir(repo.join(".turbo/tools"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("generation-"))
    );
}

#[tokio::test]
async fn unrelated_inventory_changes_reuse_node_with_unavailable_transport() {
    let mut ready = ready_node().await;
    let node = ready.plan.inventory_tool().clone();
    let sibling = sibling(&node);
    let desired = [node.clone(), sibling.clone()];
    let offline = unavailable_transport();
    assert!(!ready.store.is_current(&desired).unwrap());
    assert!(ready.store.can_reuse(&node).unwrap());
    assert!(
        ready
            .plan
            .prepare_if_needed(&ready.store, &desired, &offline)
            .await
            .unwrap()
            .is_none()
    );
    let mut staged = 0;
    assert_eq!(
        ready
            .store
            .reconcile(&desired, |tool, path| {
                assert_eq!(tool, &sibling);
                staged += 1;
                stage_sibling(tool, path)
            })
            .unwrap(),
        Outcome::Replaced
    );
    assert_eq!(staged, 1);
    assert!(ready.store.is_current(&desired).unwrap());
    assert_eq!(ready.store.current().unwrap().unwrap().tools.len(), 2);
    assert_scoped_resource(&ready.store);

    let node_only = [node.clone()];
    assert!(!ready.store.is_current(&node_only).unwrap());
    assert!(
        ready
            .plan
            .prepare_if_needed(&ready.store, &node_only, &offline)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ready
            .store
            .reconcile(&node_only, |_, _| panic!("removal must reuse Node"))
            .unwrap(),
        Outcome::Replaced
    );
    let current = ready.store.current().unwrap().unwrap();
    assert_eq!(current.tools, node_only);
    assert!(!current.bin.join("fixture").exists());
    assert_scoped_resource(&ready.store);
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        install(&mut ready.store, &ready.plan, &offline)
            .await
            .unwrap(),
        Outcome::Unchanged
    );
}

#[tokio::test]
async fn missing_generation_repairs_and_failed_repair_never_publishes_partial_state() {
    let mut ready = ready_node().await;
    let desired = [ready.plan.inventory_tool().clone()];
    let manifest = ready.repo.path().join(".turbo/tools/manifest.json");
    let before = fs::read(&manifest).unwrap();
    let generation = ready
        .store
        .current()
        .unwrap()
        .unwrap()
        .bin
        .parent()
        .unwrap()
        .to_owned();
    fs::remove_dir_all(generation).unwrap();
    assert!(!ready.store.is_current(&desired).unwrap());
    assert!(!ready.store.can_reuse(&desired[0]).unwrap());
    assert!(ready.store.current().unwrap().is_none());
    assert!(matches!(
        ready
            .plan
            .prepare_if_needed(&ready.store, &desired, &unavailable_transport())
            .await,
        Err(Error::Download(_))
    ));
    assert_eq!(fs::read(&manifest).unwrap(), before);
    assert_no_generation(ready.repo.path());
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 1);

    let prepared = ready
        .plan
        .prepare_if_needed(&ready.store, &desired, &ready.transport)
        .await
        .unwrap()
        .expect("missing generation requires Node bytes");
    assert!(
        ready
            .store
            .reconcile(&desired, |tool, path| {
                prepared.stage(tool, path)?;
                Err(turborepo_tool_install::Error::Io(io::Error::other(
                    "injected failure after copy",
                )))
            })
            .is_err()
    );
    assert_eq!(fs::read(&manifest).unwrap(), before);
    assert!(ready.store.current().unwrap().is_none());
    assert_no_generation(ready.repo.path());
    assert_eq!(
        ready
            .store
            .reconcile(&desired, |tool, path| prepared.stage(tool, path))
            .unwrap(),
        Outcome::Replaced
    );
    assert!(ready.store.is_current(&desired).unwrap());
    assert_scoped_resource(&ready.store);
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        install(&mut ready.store, &ready.plan, &unavailable_transport())
            .await
            .unwrap(),
        Outcome::Unchanged
    );
}

#[tokio::test]
async fn damaged_sibling_or_shim_requires_node_preparation_before_reconcile() {
    let mut ready = ready_node().await;
    let node = ready.plan.inventory_tool().clone();
    let desired = [node.clone(), sibling(&node)];
    ready.store.reconcile(&desired, stage_sibling).unwrap();
    for (index, damage) in ["tree", "shim"].into_iter().enumerate() {
        let current = ready.store.current().unwrap().unwrap();
        let node_bytes = fs::read(current.bin.join("node")).unwrap();
        if damage == "tree" {
            fs::write(
                current
                    .bin
                    .parent()
                    .unwrap()
                    .join("tools/fixture/bin/fixture"),
                "damaged",
            )
            .unwrap();
        } else {
            let shim = current.bin.join("fixture");
            fs::remove_file(&shim).unwrap();
            fs::write(shim, "not a symlink").unwrap();
        }
        assert_eq!(fs::read(current.bin.join("node")).unwrap(), node_bytes);
        assert!(!ready.store.can_reuse(&node).unwrap());
        assert!(matches!(
            ready
                .plan
                .prepare_if_needed(&ready.store, &desired, &unavailable_transport())
                .await,
            Err(Error::Download(_))
        ));
        let prepared = ready
            .plan
            .prepare_if_needed(&ready.store, &desired, &ready.transport)
            .await
            .unwrap()
            .expect("unhealthy generation requires Node bytes too");
        let mut staged = Vec::new();
        assert_eq!(
            ready
                .store
                .reconcile(&desired, |tool, path| {
                    staged.push(tool.id.clone());
                    if tool.id == "node" {
                        prepared.stage(tool, path)
                    } else {
                        stage_sibling(tool, path)
                    }
                })
                .unwrap(),
            Outcome::Replaced
        );
        assert_eq!(staged, ["fixture", "node"]);
        assert!(ready.store.can_reuse(&node).unwrap());
        assert!(ready.store.is_current(&desired).unwrap());
        assert_scoped_resource(&ready.store);
        assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), index + 2);
    }
}
