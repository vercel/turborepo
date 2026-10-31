use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::test_store::{Outcome, TestStore};

struct ReadyNode {
    _repo: tempfile::TempDir,
    store: TestStore,
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
    let store = TestStore::open(repo.path());
    assert_eq!(
        install(&store, &plan, &transport).await.unwrap(),
        Outcome::Replaced
    );
    ReadyNode {
        _repo: repo,
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
        vars: Default::default(),
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
pub(super) fn assert_scoped_resource(store: &TestStore) {
    let node = &store.current().unwrap().unwrap().tools;
    let node = node.iter().find(|tool| tool.id == "node").unwrap();
    assert_eq!(
        fs::read(store.tree_of(node).join(SCOPED_RESOURCE)).unwrap(),
        b"{\"name\":\"@npmcli/config\"}"
    );
}
/// Nothing a failed setup staged is left in the store.
fn assert_nothing_staged(store: &TestStore) {
    let tmp = store.store().root().join("tmp");
    assert_eq!(fs::read_dir(tmp).unwrap().count(), 0);
}

#[tokio::test]
async fn unrelated_inventory_changes_reuse_node_with_unavailable_transport() {
    let ready = ready_node().await;
    let node = ready.plan.inventory_tool().clone();
    let sibling = sibling(&node);
    let desired = [node.clone(), sibling.clone()];
    let offline = unavailable_transport();
    assert!(!ready.store.is_current(&desired).unwrap());
    assert!(ready.store.tree(&node).unwrap().is_some());
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

    let mut changed_sibling = sibling.clone();
    changed_sibling.version = "2.0.0".into();
    changed_sibling.artifact_sha256 = "c".repeat(64);
    changed_sibling.executables = BTreeMap::from([("fixture-next".into(), "bin/fixture".into())]);
    let changed_desired = [node.clone(), changed_sibling.clone()];
    assert!(!ready.store.is_current(&changed_desired).unwrap());
    assert!(ready.store.tree(&node).unwrap().is_some());
    assert!(
        ready
            .plan
            .prepare_if_needed(&ready.store, &changed_desired, &offline)
            .await
            .unwrap()
            .is_none()
    );
    let mut staged = Vec::new();
    assert_eq!(
        ready
            .store
            .reconcile(&changed_desired, |tool, path| {
                assert_eq!(tool, &changed_sibling);
                staged.push(tool.clone());
                stage_sibling(tool, path)
            })
            .unwrap(),
        Outcome::Replaced
    );
    assert_eq!(staged, [changed_sibling.clone()]);
    assert!(ready.store.is_current(&changed_desired).unwrap());
    let current = ready.store.current().unwrap().unwrap();
    assert_eq!(current.tools, [changed_sibling.clone(), node.clone()]);
    assert!(!current.bin.join("fixture").exists());
    assert_eq!(
        fs::canonicalize(current.bin.join("fixture-next")).unwrap(),
        fs::canonicalize(ready.store.tree_of(&changed_sibling).join("bin/fixture")).unwrap()
    );
    assert_scoped_resource(&ready.store);
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 1);

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
    assert!(!current.bin.join("fixture-next").exists());
    assert_scoped_resource(&ready.store);
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        install(&ready.store, &ready.plan, &offline).await.unwrap(),
        Outcome::Unchanged
    );
}

#[tokio::test]
async fn a_missing_tree_restages_and_failed_staging_never_publishes_partial_state() {
    let ready = ready_node().await;
    let desired = [ready.plan.inventory_tool().clone()];
    let tree = ready.store.tree_of(&desired[0]);
    fs::remove_dir_all(tree.parent().unwrap()).unwrap();
    assert!(ready.store.tree(&desired[0]).unwrap().is_none());
    assert!(matches!(
        ready
            .plan
            .prepare_if_needed(&ready.store, &desired, &unavailable_transport())
            .await,
        Err(Error::Download(_))
    ));
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 1);

    let prepared = ready
        .plan
        .prepare_if_needed(&ready.store, &desired, &ready.transport)
        .await
        .unwrap()
        .expect("a missing tree requires Node bytes");
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
    assert!(ready.store.tree(&desired[0]).unwrap().is_none());
    assert_nothing_staged(&ready.store);
    assert_eq!(
        ready
            .store
            .reconcile(&desired, |tool, path| prepared.stage(tool, path))
            .unwrap(),
        Outcome::Replaced
    );
    assert_scoped_resource(&ready.store);
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        install(&ready.store, &ready.plan, &unavailable_transport())
            .await
            .unwrap(),
        Outcome::Unchanged
    );
}

#[tokio::test]
async fn a_missing_environment_is_rebuilt_from_sealed_trees_without_staging() {
    let ready = ready_node().await;
    let node = ready.plan.inventory_tool().clone();
    let desired = [node.clone(), sibling(&node)];
    ready.store.reconcile(&desired, stage_sibling).unwrap();
    let bin = ready.store.bin();
    fs::remove_dir_all(bin.parent().unwrap()).unwrap();
    assert!(ready.store.current().is_err());
    assert!(
        ready
            .plan
            .prepare_if_needed(&ready.store, &desired, &unavailable_transport())
            .await
            .unwrap()
            .is_none()
    );
    ready
        .store
        .reconcile(&desired, |_, _| panic!("sealed trees are reused"))
        .unwrap();
    assert_eq!(ready.store.bin(), bin);
    assert!(bin.join("fixture").exists());
    assert_scoped_resource(&ready.store);
    assert_eq!(ready.upstream.requests.load(Ordering::SeqCst), 1);
}
