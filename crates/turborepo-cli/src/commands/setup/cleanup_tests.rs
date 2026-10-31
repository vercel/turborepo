use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use super::*;

const DAY: Duration = Duration::from_secs(86_400);

fn age(path: &Path, by: Duration) {
    let at = SystemTime::now() - by;
    fs::File::open(path).unwrap().set_modified(at).unwrap();
}

#[test]
fn selected_tools_and_shared_downloads_do_not_report_disk_use() {
    let cache = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    assert_eq!(
        f.run_with(u.origin(), Some(cache.path()), FROZEN).unwrap(),
        0
    );
    let lock = turborepo_setup::lock::Lock::parse(f.lock.to_string().as_bytes()).unwrap();
    let host = provision::platform().unwrap();
    let shared = turborepo_setup::artifact_cache::ArtifactCache::new(cache.path());
    let lines = cleanup::after_provisioning(shared.as_ref(), &lock, host);
    assert!(lines.is_empty(), "{lines:?}");
    f.no_execution();
}

#[test]
fn shared_downloads_stay_while_any_recent_setup_references_them() {
    let cache = tempfile::tempdir().unwrap();
    let (mut first, second) = (Fixture::new(), Fixture::new());
    let u = Upstream::new(&first, false, None);
    for f in [&first, &second] {
        assert_eq!(
            f.run_with(u.origin(), Some(cache.path()), FROZEN).unwrap(),
            0
        );
    }
    let mut shared = vec![first.node.clone(), first.pnpm.clone()];
    shared.sort();
    super::cache::change_node(&mut first);
    let changed = Upstream::new(&first, false, None);
    let rerun = |f: &Fixture, u: &Upstream| f.run_with(u.origin(), Some(cache.path()), FROZEN);
    assert_eq!(rerun(&first, &changed).unwrap(), 0);
    let hits = (u.hits(), changed.hits());
    let root = cache.path().join("turborepo/setup-artifacts/v1");
    for platform in fs::read_dir(root).unwrap() {
        for entry in fs::read_dir(platform.unwrap().path()).unwrap() {
            age(&entry.unwrap().path(), 100 * DAY);
        }
    }
    // Reused installs never read the cache, but their lock references still
    // count as a use. The archive no recent setup referenced is evicted.
    assert_eq!(rerun(&second, &u).unwrap(), 0);
    assert_eq!(super::cache::entries(cache.path()), shared);
    // The old Node archive is still used by another repository.
    assert_eq!(rerun(&first, &changed).unwrap(), 0);
    assert_eq!(super::cache::entries(cache.path()), shared);
    // Eviction never touches an installation or needs traffic.
    for f in [&first, &second] {
        assert_eq!(
            Installed::open(&f.owned)
                .current()
                .unwrap()
                .unwrap()
                .tools
                .len(),
            2
        );
        f.no_execution();
    }
    assert_eq!((u.hits(), changed.hits()), hits);
}

#[test]
fn cleanup_failures_are_warnings_that_never_fail_setup() {
    // A shared cache that can be neither read nor pruned.
    let cache = tempfile::tempdir().unwrap();
    let blocked = cache.path().join("turborepo/setup-artifacts");
    fs::create_dir_all(&blocked).unwrap();
    fs::write(blocked.join("v1"), "not a directory").unwrap();
    let f = Fixture::new();
    let u = Upstream::new(&f, false, None);
    assert_eq!(
        f.run_with(u.origin(), Some(cache.path()), FROZEN).unwrap(),
        0
    );
    assert_eq!(
        Installed::open(&f.owned)
            .current()
            .unwrap()
            .unwrap()
            .tools
            .len(),
        2
    );
    let lock = turborepo_setup::lock::Lock::parse(f.lock.to_string().as_bytes()).unwrap();
    let host = provision::platform().unwrap();
    let shared = turborepo_setup::artifact_cache::ArtifactCache::new(cache.path());
    let lines = cleanup::after_provisioning(shared.as_ref(), &lock, host);
    let warning = "Warning: could not clean up the shared download cache";
    assert!(
        lines.len() == 1 && lines[0].starts_with(warning),
        "{lines:?}"
    );
}
