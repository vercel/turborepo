//! Additional tools through offline setup and the artifact mirror. Local
//! fixtures only.
use turborepo_setup::mirror::{Mirror, Mirrors};

use super::{super::shared_store::unreachable_origin, *};

const OFFLINE: &[&str] = &["--frozen", "--tools-only", "--offline"];
const OFFLINE_FORCE: &[&str] = &["--frozen", "--tools-only", "--offline", "--force"];

/// A fresh clone of the seed repository with its committed lock.
fn clone_of(seed: &Fixture) -> (Fixture, LoopbackServer) {
    let (f, u) = fixture();
    let lock = fs::read(seed.owned.root().join("turbo.lock")).unwrap();
    fs::write(f.owned.root().join("turbo.lock"), lock).unwrap();
    (f, u)
}

#[test]
fn other_clones_install_additional_tools_from_the_shared_store_offline() {
    let (seed, u) = fixture();
    assert_eq!(seed.run(&u, LOCAL).unwrap(), 0);
    assert_eq!(u.hits(), 2);
    let store = seed.owned.home();
    for flags in [FROZEN, OFFLINE, OFFLINE_FORCE] {
        let (f, other) = clone_of(&seed);
        let origin = if flags == FROZEN {
            other.origin().to_owned()
        } else {
            unreachable_origin()
        };
        assert_eq!(f.run_with(&origin, store, flags).unwrap(), 0, "{flags:?}");
        assert_eq!(other.hits(), 0);
        // The clone's record now makes offline setup a trusted repeat.
        for flags in [OFFLINE, OFFLINE_FORCE] {
            assert_eq!(f.run_with(&unreachable_origin(), store, flags).unwrap(), 0);
        }
        f.no_execution();
    }
    assert_eq!(u.hits(), 2);
}

#[test]
fn offline_setup_names_each_additional_tool_missing_from_the_store() {
    let (seed, u) = fixture();
    assert_eq!(seed.run(&u, LOCAL).unwrap(), 0);
    let line = format!(
        "fixture 1.2.3 [artifact]: https://downloads.example.test/fixture-1.2.3.tar.gz (SHA-256 \
         {:x})",
        Sha256::digest(artifact())
    );
    let (f, _) = clone_of(&seed);
    let text = f
        .run_with(&unreachable_origin(), f.owned.home(), OFFLINE_FORCE)
        .unwrap_err()
        .to_string();
    assert!(
        text.contains(&line) && text.contains("node 24.0.0"),
        "{text}"
    );
    assert!(f.manifest().is_none());

    // With Node installed by another repository, only the tool is missing.
    let other = Fixture::new();
    let upstream = Upstream::new(&other, false, None);
    assert_eq!(
        other
            .run_with(upstream.origin(), f.owned.home(), FROZEN)
            .unwrap(),
        0
    );
    let text = f
        .run_with(&unreachable_origin(), f.owned.home(), OFFLINE_FORCE)
        .unwrap_err()
        .to_string();
    assert!(
        text.contains(&line) && !text.contains("node 24.0.0"),
        "{text}"
    );
    assert!(f.manifest().is_none());
    f.no_execution();
}

#[test]
fn the_artifact_mirror_serves_host_prefixed_paths_with_canonical_provenance() {
    let (f, _) = fixture();
    let routes = [
        (
            format!("/mirror/node{}", f.node_path.strip_prefix("/dist").unwrap()),
            f.node.clone(),
        ),
        (
            "/mirror/artifacts/downloads.example.test/fixture-1.2.3.tar.gz".into(),
            artifact(),
        ),
    ];
    let m = LoopbackServer::new(routes, |_| {}).unwrap();
    let mirror =
        |path: &str| Some(Mirror::loopback_for_tests(&format!("{}{path}", m.origin())).unwrap());
    let mirrors = Mirrors {
        node: mirror("/mirror/node"),
        artifact: mirror("/mirror/artifacts/"),
        ..Mirrors::default()
    };
    let policy = || -> Result<ToolDownloadPolicy, Error> {
        Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
    };
    let run = |flags: &[&str]| {
        crate::cli::dispatch_setup(&f.args(flags), |args, setup_args| {
            let mut transports = provision::Transports::configured(&policy()?, &mirrors)?;
            transports.tool_store = Some(f.owned.home().to_owned());
            run_with_policy(args, setup_args, Some(transports), f.git.clone(), policy)
        })
        .unwrap()
    };
    assert_eq!(run(LOCAL).unwrap(), 0);
    assert_eq!(m.hits(), 2);
    assert!(m.requests().iter().all(|path| path.starts_with("/mirror/")));
    // turbo.lock keeps the declared URL, never the mirror location.
    let lock = fs::read_to_string(f.owned.root().join("turbo.lock")).unwrap();
    assert!(lock.contains("https://downloads.example.test/fixture-1.2.3.tar.gz"));
    assert!(!lock.contains("127.0.0.1") && !lock.contains("/mirror"));
    assert_eq!(installed(&f), ["fixture", "node"]);
    assert_eq!(run(FROZEN).unwrap(), 0);
    assert_eq!(m.hits(), 2);
    f.no_execution();
}
