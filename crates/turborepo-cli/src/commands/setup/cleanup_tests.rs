use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use turborepo_tool_install::Store;

use super::*;

const DAY: Duration = Duration::from_secs(86_400);

fn age(path: &Path, by: Duration) {
    let at = SystemTime::now() - by;
    fs::File::open(path).unwrap().set_modified(at).unwrap();
}

fn generations(f: &Fixture) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(f.owned.root().join(".turbo/tools"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("generation-"))
        .collect();
    names.sort();
    names
}

fn selected(f: &Fixture) -> String {
    let current = Store::inspect(f.owned.root()).unwrap().unwrap();
    let generation = current.bin.parent().unwrap().file_name().unwrap();
    generation.to_string_lossy().into_owned()
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
    let mut store = Store::open(f.owned.root()).unwrap();
    let lines = cleanup::after_provisioning(&mut store, shared.as_ref(), &lock, host);
    assert!(lines.is_empty(), "{lines:?}");
    let report = cleanup::inspect(f.owned.root());
    assert!(report.is_empty(), "{report:?}");
    f.no_execution();
}

#[test]
fn replaced_tools_outlive_check_and_their_grace_then_frozen_setup_removes_them() {
    let mut f = Fixture::new();
    let first = Upstream::new(&f, false, None);
    assert_eq!(f.run(&first, FROZEN).unwrap(), 0);
    let tools = f.owned.root().join(".turbo/tools");
    let old = selected(&f);
    // However long it was selected, the grace starts at replacement.
    age(&tools.join(&old), 30 * DAY);
    super::cache::change_node(&mut f);
    let second = Upstream::new(&f, false, None);
    assert_eq!(f.run(&second, FROZEN).unwrap(), 0);
    let (new, hits) = (selected(&f), second.hits());
    let mut both = vec![old.clone(), new.clone()];
    both.sort();
    assert_eq!(generations(&f), both);
    // Past its grace, --check reports the removal but never removes.
    age(&tools.join(&old), 8 * DAY);
    let report = cleanup::inspect(f.owned.root());
    assert!(
        report[0].starts_with("turbo setup will remove "),
        "{report:?}"
    );
    let before = publication_guards::state(f.owned.root());
    assert_eq!(f.run(&second, &["--check", "--tools-only"]).unwrap(), 0);
    assert_eq!(publication_guards::state(f.owned.root()), before);
    // Frozen setup removes only what turbo.lock no longer selects.
    assert_eq!(f.run(&second, FROZEN).unwrap(), 0);
    assert_eq!(generations(&f), std::slice::from_ref(&new));
    assert_eq!(selected(&f), new);
    assert_eq!(second.hits(), hits);
    // A forced reinstall replaces the selection, which starts a new grace.
    let force = ["--frozen", "--tools-only", "--force"];
    assert_eq!(f.run(&second, &force).unwrap(), 0);
    assert!(generations(&f).len() == 2 && generations(&f).contains(&new));
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
            Store::inspect(f.owned.root()).unwrap().unwrap().tools.len(),
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
        Store::inspect(f.owned.root()).unwrap().unwrap().tools.len(),
        2
    );
    let lock = turborepo_setup::lock::Lock::parse(f.lock.to_string().as_bytes()).unwrap();
    let host = provision::platform().unwrap();
    let shared = turborepo_setup::artifact_cache::ArtifactCache::new(cache.path());
    let mut store = Store::open(f.owned.root()).unwrap();
    let lines = cleanup::after_provisioning(&mut store, shared.as_ref(), &lock, host);
    let warning = "Warning: could not clean up the shared download cache";
    assert!(
        lines.len() == 1 && lines[0].starts_with(warning),
        "{lines:?}"
    );
    // Store cleanup that fails closed is also only a warning.
    let manifest = f.owned.root().join(".turbo/tools/manifest.json");
    fs::write(manifest, br#"{"schema":1}"#).unwrap();
    let lines = cleanup::after_provisioning(&mut store, None, &lock, host);
    let warning = "Warning: could not clean up replaced managed tools";
    assert!(
        lines.len() == 1 && lines[0].starts_with(warning),
        "{lines:?}"
    );
    let report = cleanup::inspect(f.owned.root());
    assert!(
        report.len() == 1 && report[0].starts_with("Warning: "),
        "{report:?}"
    );
}
