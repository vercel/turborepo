//! Repository-declared tools through `turbo setup`, with local fixtures only.
use super::*;

const LOCAL: &[&str] = &["--tools-only", "--no-frozen"];

fn artifact() -> Vec<u8> {
    archive(
        "fixture-1.2.3",
        &[
            ("bin/fixture", "#!/bin/sh\necho fixture 1.2.3\n"),
            ("share/data.txt", "fixture resource"),
        ],
    )
}

fn tool(sha256: String) -> Value {
    let platform = serde_json::to_value(provision::platform().unwrap()).unwrap();
    json!({"version": "1.2.3", "artifacts": {platform.as_str().unwrap(): {
        "url": "https://downloads.example.test/fixture-1.2.3.tar.gz", "sha256": sha256,
        "format": "tar-gz", "rootPrefix": "fixture-1.2.3",
        "executables": {"fixture": "bin/fixture"}}}})
}

fn declare(f: &Fixture, tools: Value) {
    let config = json!({"futureFlags": {"experimentalSetup": true},
        "setup": {"additionalTools": tools}});
    fs::write(f.owned.root().join("turbo.json"), config.to_string()).unwrap();
}

/// Node plus one additional tool, served from a literal-loopback fixture.
fn fixture() -> (Fixture, LoopbackServer) {
    let mut f = Fixture::new();
    f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
    f.declare(json!({}));
    declare(
        &f,
        json!({"fixture": tool(format!("{:x}", Sha256::digest(artifact())))}),
    );
    let routes = [
        (f.node_path.clone(), f.node.clone()),
        ("/fixture-1.2.3.tar.gz".into(), artifact()),
    ];
    let u = LoopbackServer::new(routes, |_| {}).unwrap();
    (f, u)
}

fn installed(f: &Fixture) -> Vec<String> {
    let store = Installed::open(&f.owned);
    let current = store.current().unwrap().unwrap();
    current.tools.into_iter().map(|tool| tool.id).collect()
}

#[test]
fn local_setup_locks_installs_reuses_and_removes_additional_tools() {
    let (f, u) = fixture();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), 2);
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    assert_eq!(lock["tools"]["fixture"]["adapter"], "artifact");
    assert_eq!(installed(&f), ["fixture", "node"]);
    let store = Installed::open(&f.owned);
    let current = store.current().unwrap().unwrap();
    let output = std::process::Command::new(current.bin.join("fixture"))
        .output()
        .unwrap();
    assert_eq!(output.stdout, b"fixture 1.2.3\n");
    assert_eq!(
        fs::read(current.tree("fixture").join("share/data.txt")).unwrap(),
        b"fixture resource"
    );
    drop(store);

    // Frozen repeats reuse the installation and leave tracked files alone.
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    let manifest = f.manifest().unwrap();
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 2);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);
    assert_eq!(f.manifest().unwrap(), manifest);

    // Removing the declaration removes its lock entry and installation.
    declare(&f, json!({}));
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(installed(&f), ["node"]);
    let lock: Value =
        serde_json::from_slice(&fs::read(f.owned.root().join("turbo.lock")).unwrap()).unwrap();
    assert!(lock["tools"].get("fixture").is_none());
    assert_eq!(u.hits(), 2);
    f.no_execution();
}

#[test]
fn drift_and_checksum_failures_keep_the_previous_installation() {
    let (f, u) = fixture();
    assert_eq!(f.run(&u, LOCAL).unwrap(), 0);
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    let manifest = f.manifest().unwrap();

    // Same version, different digest: frozen setup names the declaration.
    declare(&f, json!({"fixture": tool("0".repeat(64))}));
    let error = f.run(&u, FROZEN).unwrap_err();
    assert!(
        matches!(&error, Error::AdditionalToolDrift(ids) if ids.contains("fixture")),
        "{error}"
    );
    assert_eq!(u.hits(), 2);
    assert_eq!(fs::read(f.owned.root().join("turbo.lock")).unwrap(), lock);

    // Local setup records the new digest, which the download then fails.
    let error = f.run(&u, LOCAL).unwrap_err();
    assert!(error.to_string().contains("\"fixture\""), "{error}");
    assert_eq!(u.hits(), 3);
    assert_eq!(f.manifest().unwrap(), manifest);
    assert_eq!(installed(&f), ["fixture", "node"]);
    f.no_execution();
}

/// One pinned archive whose `rootPrefix` decides which member runs as
/// `pkg/bin/fixture`: the whole archive stages `pkg/bin/fixture`, while
/// `rootPrefix: "pkg"` stages `pkg/pkg/bin/fixture` there instead.
fn nested() -> Vec<u8> {
    archive(
        "pkg",
        &[
            ("bin/fixture", "#!/bin/sh\necho declared\n"),
            ("pkg/bin/fixture", "#!/bin/sh\necho nested\n"),
        ],
    )
}

#[test]
fn repositories_sharing_a_store_each_stage_the_root_they_declared() {
    let bytes = nested();
    let declare_root = |f: &Fixture, root: Option<&str>| {
        let mut tool = tool(format!("{:x}", Sha256::digest(&bytes)));
        for artifact in tool["artifacts"].as_object_mut().unwrap().values_mut() {
            let artifact = artifact.as_object_mut().unwrap();
            artifact.remove("rootPrefix");
            if let Some(root) = root {
                artifact.insert("rootPrefix".into(), json!(root));
            }
            artifact.insert("executables".into(), json!({"fixture": "pkg/bin/fixture"}));
        }
        declare(f, json!({"fixture": tool}));
    };
    let (mut attacker, mut victim) = (Fixture::new(), Fixture::new());
    for f in [&mut attacker, &mut victim] {
        f.lock["tools"].as_object_mut().unwrap().remove("pnpm");
        f.declare(json!({}));
    }
    declare_root(&attacker, Some("pkg"));
    declare_root(&victim, None);
    let routes = [
        (attacker.node_path.clone(), attacker.node.clone()),
        ("/fixture-1.2.3.tar.gz".into(), bytes.clone()),
    ];
    let u = LoopbackServer::new(routes, |_| {}).unwrap();
    let store = attacker.owned.home();
    let run = |f: &Fixture| {
        let status = crate::cli::dispatch_setup(&f.args(LOCAL), |args, setup_args| {
            let mut transports = f.transports(u.origin(), None);
            transports.tool_store = Some(store.to_owned());
            run_with_policy(args, setup_args, Some(transports), f.git.clone(), || {
                Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
            })
        })
        .unwrap();
        assert_eq!(status.unwrap(), 0);
        let current = super::super::recorded_installation_for_tests(f.owned.root(), store)
            .unwrap()
            .unwrap();
        let output = std::process::Command::new(current.bin.join("fixture"))
            .output()
            .unwrap();
        let runs = String::from_utf8(output.stdout).unwrap();
        (runs, current.tree("fixture").to_owned())
    };
    let (attacker_runs, attacker_tree) = run(&attacker);
    assert_eq!(attacker_runs, "nested\n");
    let (victim_runs, victim_tree) = run(&victim);
    assert_eq!(
        victim_runs, "declared\n",
        "the victim ran the attacker's member"
    );
    assert_ne!(
        victim_tree, attacker_tree,
        "the victim reused the attacker's tree"
    );
    attacker.no_execution();
    victim.no_execution();
}

#[path = "additional_cache_tests.rs"]
mod cached;
#[path = "additional_matrix_tests.rs"]
mod matrix;
