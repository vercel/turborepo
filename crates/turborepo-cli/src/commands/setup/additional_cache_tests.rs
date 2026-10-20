//! Additional tools through the shared download cache, offline setup, and the
//! artifact mirror. Local fixtures only.
use turborepo_setup::mirror::{Mirror, Mirrors};

use super::*;

const OFFLINE: &[&str] = &["--frozen", "--tools-only", "--offline"];
const OFFLINE_FORCE: &[&str] = &["--frozen", "--tools-only", "--offline", "--force"];

fn unreachable_origin() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    origin
}

/// A fresh clone of the seed repository with its committed lock.
fn clone_of(seed: &Fixture) -> (Fixture, LoopbackServer) {
    let (f, u) = fixture();
    let lock = fs::read(seed.owned.root().join("turbo.lock")).unwrap();
    fs::write(f.owned.root().join("turbo.lock"), lock).unwrap();
    (f, u)
}

#[test]
fn other_clones_install_from_the_shared_cache_and_offline_repeats_reuse() {
    let cache = tempfile::tempdir().unwrap();
    let (seed, u) = fixture();
    assert_eq!(
        seed.run_with(u.origin(), Some(cache.path()), LOCAL)
            .unwrap(),
        0
    );
    assert_eq!(u.hits(), 2);
    // The cache holds the verified bytes, keyed by digest, never by URL.
    assert!(super::super::cache::entries(cache.path()).contains(&artifact()));
    for flags in [FROZEN, OFFLINE_FORCE] {
        let (f, other) = clone_of(&seed);
        assert_eq!(
            f.run_with(other.origin(), Some(cache.path()), flags)
                .unwrap(),
            0
        );
        assert_eq!(other.hits(), 0);
        assert_eq!(installed(&f), ["fixture", "node"]);
        // A healthy offline repeat needs neither the cache nor the network,
        // and only --force publishes again, from the trusted installed tree.
        let manifest = f.manifest();
        let empty = tempfile::tempdir().unwrap();
        let offline = |flags| f.run_with(&unreachable_origin(), Some(empty.path()), flags);
        assert_eq!(offline(OFFLINE).unwrap(), 0);
        assert_eq!(f.manifest(), manifest);
        assert_eq!(offline(OFFLINE_FORCE).unwrap(), 0);
        assert_eq!(installed(&f), ["fixture", "node"]);
        f.no_execution();
    }
}

#[test]
fn offline_misses_name_every_uncached_artifact_and_install_nothing() {
    let (seed, u) = fixture();
    assert_eq!(seed.run(&u, LOCAL).unwrap(), 0);
    let line = format!(
        "fixture 1.2.3 [artifact]: https://downloads.example.test/fixture-1.2.3.tar.gz (SHA-256 \
         {:x})",
        Sha256::digest(artifact())
    );
    let (f, _) = clone_of(&seed);
    let empty = tempfile::tempdir().unwrap();
    let text = f
        .run_with(&unreachable_origin(), Some(empty.path()), OFFLINE_FORCE)
        .unwrap_err()
        .to_string();
    assert!(
        text.contains(&line) && text.contains("node 24.0.0"),
        "{text}"
    );
    assert!(f.manifest().is_none());

    // With Node cached by another repository, only the tool is missing.
    let partial = tempfile::tempdir().unwrap();
    let other = Fixture::new();
    let upstream = Upstream::new(&other, false, None);
    assert_eq!(
        other
            .run_with(upstream.origin(), Some(partial.path()), FROZEN)
            .unwrap(),
        0
    );
    let text = f
        .run_with(&unreachable_origin(), Some(partial.path()), OFFLINE_FORCE)
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
            let transports = provision::Transports::configured(&policy()?, &mirrors, None)?;
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
