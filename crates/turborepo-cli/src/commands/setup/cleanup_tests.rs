use std::{path::Path, time::SystemTime};

use super::*;

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
            Some(first.transports(u.origin())),
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
    super::shared_store::change_node(&mut first);
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

#[test]
fn a_corrupt_record_keeps_its_state_but_not_its_tools_until_setup_replaces_it() {
    use turborepo_tool_install::GRACE;
    let f = Fixture::new();
    assert_eq!(f.run(&Upstream::new(&f, false, None), FROZEN).unwrap(), 0);
    let home = f.owned.home();
    let store = trust::user_store(Some(home), f.owned.root()).unwrap();
    let own = trust::TrustStore::new(&store, f.owned.root()).unwrap();
    let env = own.recorded_env().unwrap().unwrap().to_string();
    let record = own.path();
    fs::write(&record, "{not a record").unwrap();

    let now = SystemTime::now();
    cleanup::collect(&store, now);
    assert_eq!(names(home, "envs"), std::slice::from_ref(&env));
    cleanup::collect(&store, now + GRACE);
    // Collection cannot judge the record, so it never deletes it, but the
    // record protects nothing: its tools age out like any unused ones.
    assert_eq!(fs::read(&record).unwrap(), b"{not a record");
    assert!(names(home, "envs").is_empty());
    assert!(names(home, "trees").is_empty());

    let u = Upstream::new(&f, false, None);
    assert_eq!(f.run(&u, FROZEN).unwrap(), 0);
    assert!(u.hits() > 0, "setup reinstalled nothing");
    assert_eq!(own.recorded_env().unwrap().unwrap().to_string(), env);
    assert!(own.recorded().unwrap().is_some());
    f.no_execution();
}
