use std::{os::unix::fs::PermissionsExt, path::Path};

use turborepo_tool_install::Store;

use super::*;

fn unreachable_origin() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    origin
}

// Every file in the shared cache, failing on anything but digest entries.
pub(super) fn entries(cache: &Path) -> Vec<Vec<u8>> {
    let root = cache.join("turborepo/setup-artifacts/v1");
    let mut entries = Vec::new();
    for platform in fs::read_dir(root).unwrap() {
        for entry in fs::read_dir(platform.unwrap().path()).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = fs::read(&path).unwrap();
            assert_eq!(name, format!("{:x}", Sha256::digest(&bytes)));
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
            entries.push(bytes);
        }
    }
    entries.sort();
    entries
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

#[test]
fn two_repositories_reuse_verified_bytes_with_separate_activation() {
    let cache = tempfile::tempdir().unwrap();
    let (first, second) = (Fixture::new(), Fixture::new());
    assert_eq!(first.lock, second.lock);
    let u = Upstream::new(&first, false, None);
    assert_eq!(
        first
            .run_with(u.origin(), Some(cache.path()), FROZEN)
            .unwrap(),
        0
    );
    assert_eq!(u.hits(), 3);
    assert_eq!(
        second
            .run_with(u.origin(), Some(cache.path()), FROZEN)
            .unwrap(),
        0
    );
    // The second clone made no requests at all, not even registry metadata.
    assert_eq!(u.hits(), 3);
    let mut expected = vec![first.node.clone(), first.pnpm.clone()];
    expected.sort();
    // Only verified, non-executable archives are shared; no bin or shims.
    assert_eq!(entries(cache.path()), expected);
    for f in [&first, &second] {
        let current = Store::inspect(f.owned.root()).unwrap().unwrap();
        assert!(current.bin.starts_with(f.owned.root().join(".turbo/tools")));
        assert_eq!(
            fs::read(current.bin.join("../tools/node/resource")).unwrap(),
            b"node resource"
        );
        f.no_execution();
    }
    // Each repository keeps its own activation.
    fs::remove_dir_all(first.owned.root().join(".turbo")).unwrap();
    assert_eq!(
        Store::inspect(second.owned.root())
            .unwrap()
            .unwrap()
            .tools
            .len(),
        2
    );
}

#[test]
fn unreachable_upstream_installs_cache_hits_and_publishes_nothing_on_a_miss() {
    let cache = tempfile::tempdir().unwrap();
    let seed = Fixture::new();
    let u = Upstream::new(&seed, false, None);
    assert_eq!(
        seed.run_with(u.origin(), Some(cache.path()), FROZEN)
            .unwrap(),
        0
    );
    drop(u);
    let offline = unreachable_origin();
    let hit = Fixture::new();
    assert_eq!(
        hit.run_with(&offline, Some(cache.path()), FROZEN).unwrap(),
        0
    );
    assert_eq!(
        Store::inspect(hit.owned.root())
            .unwrap()
            .unwrap()
            .tools
            .len(),
        2
    );
    hit.no_execution();
    // A different locked Node digest is a miss: no fallback to cached bytes.
    let mut miss = Fixture::new();
    change_node(&mut miss);
    assert!(miss.run_with(&offline, Some(cache.path()), FROZEN).is_err());
    assert!(miss.manifest().is_none());
    assert_eq!(entries(cache.path()).len(), 2);
}

#[test]
fn failed_downloads_are_never_cached_and_corrupt_entries_are_replaced() {
    let cache = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let corrupt = Upstream::new(&f, true, None);
    assert!(
        f.run_with(corrupt.origin(), Some(cache.path()), FROZEN)
            .is_err()
    );
    assert!(f.manifest().is_none());
    // Verified Node is reusable; the unverified pnpm body never entered.
    assert_eq!(entries(cache.path()), std::slice::from_ref(&f.node));
    // Simulate a torn entry from an interrupted non-atomic writer.
    let node = cache
        .path()
        .join("turborepo/setup-artifacts/v1")
        .join(
            f.lock["tools"]["node"]["installation"]["artifacts"]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap(),
        )
        .join(format!("{:x}", Sha256::digest(&f.node)));
    fs::remove_file(&node).unwrap();
    fs::write(&node, &f.node[..f.node.len() / 2]).unwrap();
    let healthy = Upstream::new(&f, false, None);
    assert_eq!(
        f.run_with(healthy.origin(), Some(cache.path()), FROZEN)
            .unwrap(),
        0
    );
    assert_eq!(healthy.hits(), 3);
    let mut expected = vec![f.node.clone(), f.pnpm.clone()];
    expected.sort();
    assert_eq!(entries(cache.path()), expected);
}

#[test]
fn concurrent_repositories_share_one_cache_without_partial_entries() {
    let cache = tempfile::tempdir().unwrap();
    let repositories: Vec<_> = (0..4).map(|_| Fixture::new()).collect();
    let u = Upstream::new(&repositories[0], false, None);
    std::thread::scope(|scope| {
        for f in &repositories {
            let (origin, cache) = (u.origin(), cache.path());
            scope.spawn(move || assert_eq!(f.run_with(origin, Some(cache), FROZEN).unwrap(), 0));
        }
    });
    let mut expected = vec![repositories[0].node.clone(), repositories[0].pnpm.clone()];
    expected.sort();
    assert_eq!(entries(cache.path()), expected);
    let before = u.hits();
    let late = Fixture::new();
    assert_eq!(
        late.run_with(u.origin(), Some(cache.path()), FROZEN)
            .unwrap(),
        0
    );
    assert_eq!(u.hits(), before);
}

const OFFLINE: &[&str] = &["--frozen", "--tools-only", "--offline"];

#[test]
fn offline_setup_installs_only_installed_or_cached_bytes_without_traffic() {
    let cache = tempfile::tempdir().unwrap();
    let seed = Fixture::new();
    let u = Upstream::new(&seed, false, None);
    assert_eq!(
        seed.run_with(u.origin(), Some(cache.path()), FROZEN)
            .unwrap(),
        0
    );
    // A reachable upstream proves offline mode chooses not to use it.
    for flags in [
        OFFLINE,
        &["--tools-only", "--offline"],
        &["--tools-only", "--offline", "--force"],
    ] {
        let f = Fixture::new();
        assert_eq!(
            f.run_with(u.origin(), Some(cache.path()), flags).unwrap(),
            0
        );
        assert_eq!(
            Store::inspect(f.owned.root()).unwrap().unwrap().tools.len(),
            2
        );
        f.no_execution();
    }
    // Healthy installed tools need no cache entries at all.
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(
        seed.run_with(u.origin(), Some(empty.path()), OFFLINE)
            .unwrap(),
        0
    );
    assert_eq!(u.hits(), 3);
}

#[test]
fn offline_misses_list_every_missing_artifact_and_publish_nothing() {
    let f = Fixture::new();
    let node = format!(
        "node 24.0.0 [distribution]: https://nodejs.org{}",
        f.node_path
    );
    let pnpm = "pnpm 10.0.0 [package]: https://registry.npmjs.org/pnpm/-/pnpm-10.0.0.tgz";
    let empty = tempfile::tempdir().unwrap();
    for cache in [None, Some(empty.path())] {
        let text = f
            .run_with(&unreachable_origin(), cache, OFFLINE)
            .unwrap_err()
            .to_string();
        assert!(text.contains(&node) && text.contains(pnpm), "{text}");
        assert!(
            text.contains(&format!("{:x}", Sha256::digest(&f.pnpm))),
            "{text}"
        );
        assert!(f.manifest().is_none());
    }
    // Only the uncached artifact is listed; cached Node is not installed alone.
    let partial = tempfile::tempdir().unwrap();
    let corrupt = Upstream::new(&f, true, None);
    assert!(
        f.run_with(corrupt.origin(), Some(partial.path()), FROZEN)
            .is_err()
    );
    let hits = corrupt.hits();
    let text = f
        .run_with(corrupt.origin(), Some(partial.path()), OFFLINE)
        .unwrap_err()
        .to_string();
    assert!(
        !text.contains("node 24.0.0") && text.contains(pnpm),
        "{text}"
    );
    assert_eq!(corrupt.hits(), hits);
    assert!(f.manifest().is_none());
    f.no_execution();
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
fn offline_force_reinstalls_from_cached_or_verified_installed_bytes() {
    const FORCE: &[&str] = &["--frozen", "--tools-only", "--offline", "--force"];
    let cache = tempfile::tempdir().unwrap();
    let (seed, f) = (Fixture::new(), Fixture::new());
    let u = Upstream::new(&f, false, None);
    // Another clone seeds the cache; this one installs without it.
    assert_eq!(
        seed.run_with(u.origin(), Some(cache.path()), FROZEN)
            .unwrap(),
        0
    );
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    let hits = u.hits();
    let pnpm = format!(
        "turborepo/setup-artifacts/v1/any/{:x}",
        Sha256::digest(&f.pnpm)
    );
    fs::remove_file(cache.path().join(pnpm)).unwrap();
    // Cached Node with installed-only pnpm, then installed-only for both.
    for cache in [Some(cache.path()), None] {
        let before = f.manifest().unwrap();
        assert_eq!(f.run_with(&unreachable_origin(), cache, FORCE).unwrap(), 0);
        assert_ne!(f.manifest().unwrap(), before);
        let current = Store::inspect(f.owned.root()).unwrap().unwrap();
        assert_eq!(current.tools.len(), 2);
    }
    // Neither cached nor healthy: every artifact is listed, nothing published.
    let current = Store::inspect(f.owned.root()).unwrap().unwrap();
    turborepo_setup::test_support::damage(
        &current.bin.join("../tools/pnpm/dist/resource"),
        "damage",
    );
    let before = f.manifest().unwrap();
    let text = f
        .run_with(&unreachable_origin(), None, FORCE)
        .unwrap_err()
        .to_string();
    let both =
        text.contains("node 24.0.0 [distribution]") && text.contains("pnpm 10.0.0 [package]");
    assert!(both, "{text}");
    assert_eq!(f.manifest().unwrap(), before);
    assert_eq!(u.hits(), hits);
    f.no_execution();
}
