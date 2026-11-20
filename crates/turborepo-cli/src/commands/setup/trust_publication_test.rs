use super::*;

#[test]
fn unsafe_external_state_after_download_blocks_publication_and_setup_repairs_it() {
    let f = Fixture::new();
    let repos = f.owned.home().join("repos");
    let state = turborepo_tool_install::UserStore::repo_in(
        f.owned.home(),
        &f.owned.root().canonicalize().unwrap(),
    );
    let u = LoopbackServer::node_pnpm(
        f.node_path.clone(),
        f.node.clone(),
        f.pnpm.clone(),
        false,
        move |request| {
            if request == "/pnpm/-/pnpm-10.0.0.tgz" {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
                fs::create_dir(state.join("record.json")).unwrap();
            }
        },
    )
    .unwrap();
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 3);
    assert!(
        Installed::open(&f.owned)
            .current()
            .unwrap_or(None)
            .is_none()
    );
    let blocked: Vec<_> = fs::read_dir(&repos)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("record.json"))
        .collect();
    assert!(blocked.iter().all(|path| path.is_dir()));
    for path in &blocked {
        fs::remove_dir(path).unwrap();
    }
    let repair = Upstream::new(&f, false, None);
    f.run(&repair, FROZEN).unwrap();
    // Trust preflight failed before anything was published, so the repair
    // freshly verifies every artifact.
    assert_eq!(repair.hits(), 3);
    assert!(Installed::open(&f.owned).current().unwrap().is_some());
    f.no_execution();
}
