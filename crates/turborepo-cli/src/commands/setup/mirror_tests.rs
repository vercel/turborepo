//! Path-prefixed mirror fixtures: canonical lock provenance, verified bytes,
//! and errors that never echo mirror locations.
use turborepo_setup::mirror::{Mirror, Mirrors};

use super::*;

// Serve the standard resolution fixture beneath two mirror bases.
fn mirrored(routes: Vec<(String, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    routes
        .into_iter()
        .map(|(path, body)| match path.strip_prefix("/dist") {
            Some(rest) => (format!("/mirror/node{rest}"), body),
            None => (format!("/mirror/npm{path}"), body),
        })
        .collect()
}

fn run_mirrored(
    f: &Fixture,
    u: &LoopbackServer,
    prefix: &str,
    flags: &[&str],
) -> Result<i32, Error> {
    let mirror = |path: &str| {
        Some(Mirror::loopback_for_tests(&format!("{}{prefix}{path}", u.origin())).unwrap())
    };
    let mirrors = Mirrors {
        node: mirror("/node/"),
        registry: mirror("/npm"),
    };
    let policy = || -> Result<OfficialSourcePolicy, Error> {
        Ok(f.owned.policy_at(&f.owned.root().join("apps/web/src"))?)
    };
    crate::cli::dispatch_setup(&f.args(flags), |args, setup_args| {
        let transports = provision::Transports::configured(&policy()?, &mirrors, None)?;
        run_with_policy(args, setup_args, Some(transports), policy)
    })
    .unwrap()
}

fn chain(error: &Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

#[test]
fn mirrors_create_and_install_locks_with_canonical_provenance() {
    let (official, mirrored_fixture) = (fresh(), fresh());
    let u = LoopbackServer::new(routes(&official, "24.0.0", "10.0.0"), |_| {}).unwrap();
    assert_eq!(official.run(&u, LOCAL).unwrap(), 0);
    let routes = mirrored(routes(&mirrored_fixture, "24.0.0", "10.0.0"));
    let m = LoopbackServer::new(routes, |_| {}).unwrap();
    assert_eq!(
        run_mirrored(&mirrored_fixture, &m, "/mirror", LOCAL).unwrap(),
        0
    );
    assert_eq!(m.hits(), 7);
    assert!(m.requests().iter().all(|path| path.starts_with("/mirror/")));
    // Transport locations never reach the committed lock.
    let lock = fs::read(mirrored_fixture.owned.root().join("turbo.lock")).unwrap();
    assert_eq!(
        lock,
        fs::read(official.owned.root().join("turbo.lock")).unwrap()
    );
    assert!(!String::from_utf8(lock).unwrap().contains("127.0.0.1"));
    assert_eq!(
        run_mirrored(&mirrored_fixture, &m, "/mirror", FROZEN).unwrap(),
        0
    );
    assert_eq!(m.hits(), 7);
    untouched(&mirrored_fixture);
}

#[test]
fn missing_mirror_index_creates_no_lock_or_storage() {
    let f = fresh();
    let mut routes = mirrored(routes(&f, "24.0.0", "10.0.0"));
    routes.retain(|(path, _)| path != "/mirror/node/index.json");
    let m = LoopbackServer::new(routes, |_| {}).unwrap();
    assert!(run_mirrored(&f, &m, "/mirror", LOCAL).is_err());
    assert_eq!(m.requests(), ["/mirror/node/index.json"]);
    assert!(!f.owned.root().join("turbo.lock").exists());
    assert!(!f.owned.root().join(".turbo").exists());
    untouched(&f);
}

#[test]
fn mirror_checksum_mismatch_or_missing_artifact_installs_nothing() {
    for tampered in [true, false] {
        let f = Fixture::new();
        let path = format!("/mirror/node{}", f.node_path.strip_prefix("/dist").unwrap());
        let routes = tampered.then(|| (path.clone(), b"tampered".to_vec()));
        let m = LoopbackServer::new(routes, |_| {}).unwrap();
        let error = run_mirrored(&f, &m, "/mirror", FROZEN).unwrap_err();
        let expected = if tampered { "does not match" } else { "404" };
        assert!(chain(&error).contains(expected), "{}", chain(&error));
        // The locked digest stops the cohort before any pnpm traffic.
        assert_eq!(m.requests(), [path]);
        assert!(f.manifest().is_none());
        f.no_execution();
    }
}

#[test]
fn mirror_failures_never_echo_mirror_locations_or_path_tokens() {
    let f = Fixture::new();
    let m = LoopbackServer::new([], |_| {}).unwrap();
    let error = chain(&run_mirrored(&f, &m, "/s3cret-token", FROZEN).unwrap_err());
    assert!(error.contains("404"), "{error}");
    assert!(
        !error.contains("s3cret") && !error.contains("127.0.0.1"),
        "{error}"
    );
    assert!(m.requests()[0].starts_with("/s3cret-token/node/v24.0.0/"));
}
