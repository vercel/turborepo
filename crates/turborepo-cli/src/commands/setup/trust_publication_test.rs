use super::*;

#[test]
fn unsafe_external_state_after_download_blocks_publication_and_setup_repairs_it() {
    let f = Fixture::new();
    let blocked = f.owned.home().join("turborepo");
    let blocker = blocked.clone();
    let u = LoopbackServer::node_pnpm(
        f.node_path.clone(),
        f.node.clone(),
        f.pnpm.clone(),
        false,
        move |request| {
            if request == "/pnpm/-/pnpm-10.0.0.tgz" {
                fs::write(&blocker, "cannot publish user state here").unwrap();
            }
        },
    )
    .unwrap();
    assert!(f.run(&u, FROZEN).is_err());
    assert_eq!(u.hits(), 3);
    assert!(
        turborepo_tool_install::Store::inspect(f.owned.root())
            .unwrap()
            .is_none()
    );
    let before = f.manifest();
    assert_eq!(
        fs::read(&blocked).unwrap(),
        b"cannot publish user state here"
    );
    fs::remove_file(&blocked).unwrap();
    let repair = Upstream::new(&f, false, None);
    f.run(&repair, FROZEN).unwrap();
    assert_eq!(repair.hits(), 3); // Interrupted preparation is freshly verified.
    assert_ne!(f.manifest(), before);
    assert_eq!(
        fs::read_dir(blocked.join("setup-roots")).unwrap().count(),
        1
    );
    f.no_execution();
}
