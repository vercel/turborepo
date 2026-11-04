//! Repositories sharing one tool store, online and offline.
use std::path::Path;

use super::*;

const OFFLINE: &[&str] = &["--frozen", "--tools-only", "--offline"];

/// A loopback origin that refuses every connection.
pub(super) fn unreachable_origin() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    origin
}

pub(super) fn change_node(f: &mut Fixture) {
    let prefix = f
        .node_path
        .rsplit('/')
        .next()
        .unwrap()
        .trim_end_matches(".tar.gz");
    f.node = archive(prefix, &[("bin/node", "exit 91\n"), ("resource", "new")]);
    let artifacts = f.lock["tools"]["node"]["installation"]["artifacts"]
        .as_object_mut()
        .unwrap();
    artifacts.values_mut().next().unwrap()["distribution"]["sha256"] =
        json!(format!("{:x}", Sha256::digest(&f.node)));
    f.save();
}

/// The environment `f`'s record names in the tool store at `store`.
fn env(store: &Path, f: &Fixture) -> Option<String> {
    let store = trust::user_store(Some(store), f.owned.root()).unwrap();
    trust::TrustStore::new(&store, f.owned.root())
        .unwrap()
        .recorded_env()
        .unwrap()
        .map(|env| env.to_string())
}

#[test]
fn a_second_repository_with_the_same_lock_downloads_nothing_and_shares_the_environment() {
    let (first, second) = (Fixture::new(), Fixture::new());
    assert_eq!(first.lock, second.lock);
    let u = Upstream::new(&first, false, None);
    assert_eq!(first.run(&u, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), 3);
    let store = first.owned.home();
    assert_eq!(second.run_with(u.origin(), store, FROZEN).unwrap(), 0);
    // No request at all: every tree is already sealed in the store.
    assert_eq!(u.hits(), 3);
    assert_eq!(env(store, &first), env(store, &second));
    // Each repository's editor link resolves to that one environment.
    let bin = |f: &Fixture| fs::canonicalize(f.owned.root().join(".turbo/bin")).unwrap();
    assert_eq!(bin(&first), bin(&second));
    assert!(bin(&second).starts_with(fs::canonicalize(store).unwrap()));
    assert!(!second.owned.root().join(".turbo/tools").exists());
    first.no_execution();
    second.no_execution();
}

#[test]
fn concurrent_repositories_share_one_store() {
    let repositories: Vec<_> = (0..4).map(|_| Fixture::new()).collect();
    let store = repositories[0].owned.home();
    let u = Upstream::new(&repositories[0], false, None);
    std::thread::scope(|scope| {
        for f in &repositories {
            let origin = u.origin();
            scope.spawn(move || assert_eq!(f.run_with(origin, store, FROZEN).unwrap(), 0));
        }
    });
    let shared = env(store, &repositories[0]);
    assert!(shared.is_some());
    for f in &repositories {
        assert_eq!(env(store, f), shared);
        f.no_execution();
    }
    let before = u.hits();
    let late = Fixture::new();
    assert_eq!(late.run_with(u.origin(), store, FROZEN).unwrap(), 0);
    assert_eq!(u.hits(), before);
}

#[test]
fn offline_setup_in_a_second_repository_installs_from_the_store_without_network() {
    let seed = Fixture::new();
    let u = Upstream::new(&seed, false, None);
    assert_eq!(seed.run(&u, FROZEN).unwrap(), 0);
    let store = seed.owned.home();
    for flags in [
        OFFLINE,
        &["--tools-only", "--offline"],
        &["--tools-only", "--offline", "--force"],
    ] {
        let f = Fixture::new();
        assert_eq!(
            f.run_with(&unreachable_origin(), store, flags).unwrap(),
            0,
            "{flags:?}"
        );
        assert_eq!(env(store, &f), env(store, &seed));
        assert!(f.owned.root().join(".turbo/bin").exists());
        // The new record makes the next offline setup a trusted repeat.
        assert_eq!(
            f.run_with(&unreachable_origin(), store, OFFLINE).unwrap(),
            0
        );
        f.no_execution();
    }
    assert_eq!(
        seed.run_with(&unreachable_origin(), store, OFFLINE)
            .unwrap(),
        0
    );
    assert_eq!(u.hits(), 3);
}

#[test]
fn offline_setup_names_each_tool_missing_from_the_store_and_publishes_nothing() {
    let f = Fixture::new();
    let node = format!(
        "node 24.0.0 [distribution]: https://nodejs.org{} (SHA-256 {:x})",
        f.node_path,
        Sha256::digest(&f.node)
    );
    let pnpm = format!(
        "pnpm 10.0.0 [package]: https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz (SHA-256 {:x})",
        Sha256::digest(&f.pnpm)
    );
    let error = f
        .run_with(&unreachable_origin(), f.owned.home(), OFFLINE)
        .unwrap_err();
    assert!(matches!(error, Error::OfflineMissing(_)), "{error}");
    let text = error.to_string();
    assert!(text.contains(&node) && text.contains(&pnpm), "{text}");
    assert!(f.manifest().is_none());
    assert!(!f.owned.root().join(".turbo").exists());

    // Another repository installed pnpm, but not this lock's Node.
    let seed = Fixture::new();
    let u = Upstream::new(&seed, false, None);
    assert_eq!(seed.run(&u, FROZEN).unwrap(), 0);
    let mut changed = Fixture::new();
    change_node(&mut changed);
    let store = seed.owned.home();
    let text = changed
        .run_with(&unreachable_origin(), store, OFFLINE)
        .unwrap_err()
        .to_string();
    let node = format!(
        "node 24.0.0 [distribution]: https://nodejs.org{}",
        changed.node_path
    );
    assert!(text.contains(&node), "{text}");
    assert!(!text.contains("pnpm 10.0.0"), "{text}");
    assert_eq!(env(store, &changed), None);
    assert!(!changed.owned.root().join(".turbo").exists());
    assert_eq!(u.hits(), 3);
    changed.no_execution();
}

#[test]
fn offline_never_creates_refreshes_or_re_resolves_a_lock() {
    for case in 0..3 {
        let f = Fixture::new();
        let u = Upstream::new(&f, false, None);
        let flags: &[&str] = match case {
            0 => {
                fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
                &["--tools-only", "--no-frozen", "--offline"]
            }
            1 => {
                fs::write(f.owned.root().join(".nvmrc"), "26.0.0").unwrap();
                &["--tools-only", "--no-frozen", "--offline"]
            }
            _ => &["--tools-only", "--no-frozen", "--offline", "--update-lock"],
        };
        assert!(
            matches!(f.run(&u, flags), Err(Error::Unsupported(_))),
            "{case}"
        );
        assert_eq!(u.hits(), 0);
        assert!(!f.owned.root().join(".turbo").exists());
        assert_eq!(f.owned.root().join("turbo.lock").exists(), case != 0);
    }
}

#[test]
fn offline_setup_rehashes_installed_trees_and_names_each_damaged_one() {
    const FORCE: &[&str] = &["--frozen", "--tools-only", "--offline", "--force"];
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let hits = u.hits();
    let store = f.owned.home();
    for _ in 0..2 {
        let before = f.manifest().unwrap();
        assert_eq!(f.run_with(&unreachable_origin(), store, FORCE).unwrap(), 0);
        assert_eq!(f.manifest().unwrap(), before);
    }
    let current = Installed::open(&f.owned).current().unwrap().unwrap();
    turborepo_setup::test_support::damage(&current.tree("pnpm").join("dist/resource"), "damage");
    let before = f.manifest().unwrap();
    for flags in [OFFLINE, FORCE] {
        let text = f
            .run_with(&unreachable_origin(), store, flags)
            .unwrap_err()
            .to_string();
        assert!(text.contains("pnpm 10.0.0 [package]"), "{flags:?}: {text}");
        assert!(!text.contains("node 24.0.0 ["), "{flags:?}: {text}");
        assert_eq!(f.manifest().unwrap(), before);
    }
    assert_eq!(u.hits(), hits);
    f.no_execution();
}
