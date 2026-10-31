use std::{
    sync::mpsc,
    time::{Duration, SystemTime},
};

use super::*;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn version(id: &str, version: &str) -> Tool {
    Tool {
        version: version.into(),
        ..tool(id)
    }
}

/// Repositories that are roots, each naming its current environment; every
/// other repository directory has been dropped.
fn try_collect(store: &UserStore, roots: &[(&Path, &EnvKey)], at: SystemTime) -> Option<Collected> {
    let roots: BTreeMap<PathBuf, EnvKey> = roots
        .iter()
        .map(|(repo, env)| (store.repo(&fs::canonicalize(repo).unwrap()), (*env).clone()))
        .collect();
    store
        .collect(at, |dir| match roots.get(dir) {
            Some(env) => RepoState::Root(env.clone()),
            None => RepoState::Dropped,
        })
        .unwrap()
}

/// [`try_collect`] that waits for a round that runs. A round is skipped while
/// anything holds the store's lock, including a child process another test
/// spawned while this test held a lock: the child keeps the lock until its
/// exec completes.
pub(super) fn collect(store: &UserStore, roots: &[(&Path, &EnvKey)], at: SystemTime) -> Collected {
    for _ in 0..500 {
        if let Some(collected) = try_collect(store, roots, at) {
            return collected;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the tool store stayed locked for 5 seconds");
}

fn state(store: &UserStore) -> BTreeMap<String, u64> {
    serde_json::from_slice(&fs::read(store.root().join("gc-state.json")).unwrap()).unwrap()
}

fn secs(at: SystemTime) -> u64 {
    at.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

#[test]
fn a_replaced_environment_outlives_the_grace_period_and_shared_trees_outlive_it() {
    let f = fixture();
    let repo = repo();
    let root = fs::canonicalize(repo.path()).unwrap();
    let (old, new, pnpm) = (
        version("node", "1.0.0"),
        version("node", "2.0.0"),
        tool("pnpm"),
    );
    let first = ensure(
        &f.store,
        repo.path(),
        &[old.clone(), pnpm.clone()],
        &Cell::new(0),
    );
    // A task started from the first environment holds its bin path.
    let running = f
        .store
        .activate(&root, &first.env)
        .unwrap()
        .bin()
        .join("node");
    let second = ensure(
        &f.store,
        repo.path(),
        &[new.clone(), pnpm.clone()],
        &Cell::new(0),
    );
    let roots = [(repo.path(), &second.env)];
    let now = SystemTime::now();

    for at in [now, now + GRACE - DAY] {
        let collected = collect(&f.store, &roots, at);
        assert_eq!(collected.removed.entries, 0);
        // The old environment and the node tree only it uses.
        assert_eq!(collected.retained.entries, 2);
        assert!(Command::new(&running).output().unwrap().status.success());
        assert!(f.store.sealed_tree(&old).unwrap().is_some());
    }

    let collected = collect(&f.store, &roots, now + GRACE);
    assert_eq!(collected.removed.entries, 2);
    assert_eq!(collected.retained, Usage::default());
    assert!(collected.removed.bytes > 0);
    assert!(!running.exists());
    assert!(f.store.activate(&root, &first.env).is_err());
    assert!(f.store.sealed_tree(&old).unwrap().is_none());
    assert!(f.store.sealed_tree(&pnpm).unwrap().is_some());
    assert!(f.store.sealed_tree(&new).unwrap().is_some());
    let current = f.store.activate(&root, &second.env).unwrap();
    assert!(
        Command::new(current.bin().join("node"))
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new(current.stable_bin().join("pnpm"))
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn collection_skips_without_waiting_while_a_setup_holds_the_store() {
    let f = fixture();
    let repo = repo();
    let tools = [tool("node")];
    let unused = ensure(&f.store, repo.path(), &tools, &Cell::new(0));
    let session = f.store.begin_setup(repo.path(), Reuse::Sealed).unwrap();
    let (sender, receiver) = mpsc::channel();
    let store = f.store.clone();
    let late = SystemTime::now() + GRACE * 100;
    std::thread::spawn(move || sender.send(try_collect(&store, &[], late)).unwrap());
    let skipped = receiver.recv_timeout(Duration::from_secs(10));
    assert_eq!(skipped, Ok(None), "collection did not skip a held store");
    let root = fs::canonicalize(repo.path()).unwrap();
    assert!(f.store.activate(&root, &unused.env).is_ok());
    drop(session);
    // Collection still never deletes anything it first sees unreferenced.
    let ran = collect(&f.store, &[], late);
    assert!(ran.removed.entries == 0 && ran.retained.entries > 0);
}

#[test]
fn trash_left_by_a_crashed_collection_is_removed_by_the_next() {
    let f = fixture();
    let repo = repo();
    ensure(&f.store, repo.path(), &[tool("node")], &Cell::new(0));
    let tmp = f.store.root().join("tmp");
    let entry = tmp.join("trash-crashed/entry/root/bin");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("tool"), "bytes").unwrap();
    fs::set_permissions(entry.join("tool"), fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o555)).unwrap();
    fs::write(tmp.join(".tmp-record"), "half-written").unwrap();
    collect(&f.store, &[], SystemTime::now());
    assert_eq!(fs::read_dir(&tmp).unwrap().count(), 0);
}

#[test]
fn the_grace_clock_starts_once_and_restarts_only_after_a_reference() {
    let f = fixture();
    let repo = repo();
    let first = ensure(
        &f.store,
        repo.path(),
        &[version("node", "1.0.0")],
        &Cell::new(0),
    );
    let second = ensure(
        &f.store,
        repo.path(),
        &[version("node", "2.0.0")],
        &Cell::new(0),
    );
    let key = format!("envs/{}", first.env);
    let t0 = SystemTime::now();
    collect(&f.store, &[(repo.path(), &second.env)], t0);
    assert_eq!(state(&f.store).get(&key), Some(&secs(t0)));
    collect(&f.store, &[(repo.path(), &second.env)], t0 + DAY);
    assert_eq!(state(&f.store).get(&key), Some(&secs(t0)));

    // Referenced again, then dropped again: the earlier clock is gone.
    collect(&f.store, &[(repo.path(), &first.env)], t0 + DAY * 2);
    assert_eq!(state(&f.store).get(&key), None);
    let dropped = t0 + DAY * 3;
    collect(&f.store, &[(repo.path(), &second.env)], dropped);
    assert_eq!(state(&f.store).get(&key), Some(&secs(dropped)));
    collect(&f.store, &[(repo.path(), &second.env)], t0 + GRACE + DAY);
    let root = fs::canonicalize(repo.path()).unwrap();
    assert!(f.store.activate(&root, &first.env).is_ok());
    collect(&f.store, &[(repo.path(), &second.env)], dropped + GRACE);
    assert!(f.store.activate(&root, &first.env).is_err());
    assert_eq!(state(&f.store).get(&key), None);
}
