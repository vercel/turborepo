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

/// Every entry in one of the tool store's directories, by name.
fn names(store: &Path, kind: &str) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(store.join(kind))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn a_deleted_repositorys_tools_and_state_go_only_after_the_grace_period() {
    use turborepo_tool_install::GRACE;
    let (mut first, second) = (Fixture::new(), Fixture::new());
    let u = Upstream::new(&first, false, None);
    assert_eq!(first.run(&u, FROZEN).unwrap(), 0);
    // Another clone of the same user, in `first`'s tool store.
    let code = crate::cli::dispatch_setup(&second.args(FROZEN), |args, setup| {
        run_with_policy(
            args,
            setup,
            Some(first.transports(u.origin(), None)),
            second.git.clone(),
            || {
                Ok(second
                    .owned
                    .policy_at(&second.owned.root().join("apps/web/src"))?)
            },
        )
    })
    .unwrap()
    .unwrap();
    assert_eq!(code, 0);
    let home = &first.owned.home().to_owned();
    let store = trust::user_store(Some(home), first.owned.root()).unwrap();
    let env = |f: &Fixture| {
        trust::TrustStore::new(&store, f.owned.root())
            .unwrap()
            .recorded_env()
            .unwrap()
            .unwrap()
            .to_string()
    };
    let shared = env(&first);
    super::cache::change_node(&mut first);
    assert_eq!(
        first
            .run(&Upstream::new(&first, false, None), FROZEN)
            .unwrap(),
        0
    );
    let current = env(&first);
    assert_ne!(current, shared);
    // The other clone still selects the first environment.
    assert!(cleanup::collect(&store, SystemTime::now() + GRACE * 2).is_empty());
    assert_eq!(names(home, "envs"), {
        let mut both = vec![shared.clone(), current.clone()];
        both.sort();
        both
    });

    let state = store.repo(second.owned.root());
    fs::remove_dir_all(second.owned.root()).unwrap();
    let now = SystemTime::now();
    let lines = cleanup::collect(&store, now);
    assert!(
        lines.len() == 1 && lines[0].starts_with("Keeping ") && lines[0].contains("for 7 days"),
        "{lines:?}"
    );
    assert!(names(home, "envs").contains(&shared));
    assert!(state.join("record.json").is_file());
    assert_eq!(names(home, "trees").len(), 3);

    let lines = cleanup::collect(&store, now + GRACE);
    assert!(
        lines.len() == 1 && lines[0].starts_with("Removed ") && lines[0].ends_with("selects."),
        "{lines:?}"
    );
    assert_eq!(names(home, "envs"), [current]);
    assert!(!state.exists());
    // Only the replaced Node tree went; pnpm is shared with the current one.
    assert_eq!(names(home, "trees").len(), 2);
    let own = trust::TrustStore::new(&store, first.owned.root()).unwrap();
    assert!(own.recorded().unwrap().is_some());
    assert!(cleanup::collect(&store, now + GRACE * 3).is_empty());
    first.no_execution();
}
