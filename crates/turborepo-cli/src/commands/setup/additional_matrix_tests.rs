//! The rest of the Unix additional-tool matrix: read-only plan and check,
//! frozen lock stability, unsupported platforms, and collisions between
//! declared tools. Local fixtures only.
use super::{super::checks::snapshot, *};

const CHECK: &[&str] = super::super::checks::CHECK;

fn plan(f: &Fixture) -> String {
    let discovery = root::Discovery::capture(&f.args(&["--plan", "--tools-only"])).unwrap();
    let sources = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
    plan::inspect(&discovery, sources, false, || {
        Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
    })
    .unwrap()
}

fn lock(f: &Fixture) -> Vec<u8> {
    fs::read(f.owned.root().join("turbo.lock")).unwrap()
}

#[test]
fn plan_and_check_are_read_only_and_check_rehashes_adjacent_resources() {
    let (f, u) = fixture();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let before = snapshot(f.owned.root().parent().unwrap());
    let report = plan(&f);
    for line in [
        "fixture 1.2.3: reuse (healthy installation)",
        "  Declaration: turbo.json#/setup/additionalTools/fixture (1.2.3)",
        "  Source: https://downloads.example.test/fixture-1.2.3.tar.gz",
    ] {
        assert!(report.lines().any(|l| l == line), "{line}\n{report}");
    }
    assert_eq!(snapshot(f.owned.root().parent().unwrap()), before);
    assert_eq!(f.check(CHECK).unwrap(), 0);

    // --check re-hashes the whole tree, so damage beside the executable is
    // caught, and it never repairs anything.
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    let current = store.current().unwrap().unwrap();
    let tool = current.tools.iter().find(|t| t.id == "fixture").unwrap();
    let data = current.tool_tree(tool).unwrap().join("share/data.txt");
    drop(store);
    fs::write(&data, "tampered").unwrap();
    let error = f.check(CHECK).unwrap_err();
    assert!(
        matches!(
            error,
            Error::Activation(turborepo_setup::activation::Error::DamagedInventory(_))
        ),
        "{error}"
    );
    assert!(plan(&f).contains("fixture 1.2.3: repair (damaged installation)"));
    assert_eq!(u.hits(), 2);
    // Frozen setup repairs it from the unchanged lock.
    let locked = lock(&f);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(f.check(CHECK).unwrap(), 0);
    assert_eq!(lock(&f), locked);
    f.no_execution();
}

#[test]
fn frozen_setup_never_adds_platforms_and_unsupported_ones_fail_before_traffic() {
    let (f, u) = fixture();
    // Also declared for a platform this host never installs.
    let mut tools = json!({"fixture": tool(format!("{:x}", Sha256::digest(artifact())))});
    let other = json!({"url": "https://downloads.example.test/fixture-1.2.3.zip",
        "sha256": "1".repeat(64), "format": "zip", "rootPrefix": "fixture-1.2.3",
        "executables": {"fixture": "bin/fixture"}});
    tools["fixture"]["artifacts"]["linux-arm64-musl"] = other;
    declare(&f, tools.clone());
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let locked = lock(&f);
    let value: Value = serde_json::from_slice(&locked).unwrap();
    let artifacts = &value["tools"]["fixture"]["installation"]["artifacts"];
    assert_eq!(artifacts.as_object().unwrap().len(), 2, "{value}");
    for _ in 0..2 {
        assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
        assert_eq!(lock(&f), locked);
    }
    assert_eq!(u.hits(), 2);

    // A clone on a platform the declaration lacks fails, naming the tool and
    // platform, before any request or storage.
    let (g, v) = fixture();
    let host = serde_json::to_value(provision::platform().unwrap()).unwrap();
    let mut elsewhere = tools;
    elsewhere["fixture"]["artifacts"]
        .as_object_mut()
        .unwrap()
        .remove(host.as_str().unwrap());
    declare(&g, elsewhere);
    let error = format!("{:?}", g.run(&v, LOCAL).unwrap_err());
    assert!(
        error.contains(&format!(
            "MissingTarget {{ tool: \"fixture\", platform: {host} }}"
        )),
        "{error}"
    );
    assert_eq!(v.hits(), 0);
    assert!(g.manifest().is_none());
    g.no_execution();
}

#[test]
fn executables_shared_between_additional_tools_are_rejected_before_traffic() {
    let (f, u) = fixture();
    let digest = format!("{:x}", Sha256::digest(artifact()));
    let mut second = tool(digest.clone());
    let host = serde_json::to_value(provision::platform().unwrap()).unwrap();
    second["artifacts"][host.as_str().unwrap()]["executables"] = json!({"FIXTURE": "bin/fixture"});
    declare(&f, json!({"fixture": tool(digest), "other": second}));
    let locked = lock(&f);
    let error = format!("{:?}", f.run(&u, LOCAL).unwrap_err());
    assert!(error.contains("is also declared by"), "{error}");
    assert_eq!(lock(&f), locked);
    assert_eq!(u.hits(), 0);
    assert!(f.manifest().is_none());
}
