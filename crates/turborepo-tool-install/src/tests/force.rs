use super::*;

#[test]
fn force_stages_every_healthy_tool_without_changing_readiness_truth() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [tool("node"), tool("pnpm")];
    store.reconcile(&desired, populate).unwrap();
    let before = manifest(&store);
    let old = store.current().unwrap().unwrap().bin;
    for tool in &desired {
        assert!(store.can_reuse(tool).unwrap());
    }
    let mut staged = Vec::new();
    assert_eq!(
        store
            .force_reconcile_checked(
                &desired,
                |tool, root| {
                    staged.push(tool.id.clone());
                    populate(tool, root)
                },
                || Ok(())
            )
            .unwrap(),
        Outcome::Replaced
    );
    assert_eq!(staged, ["node", "pnpm"]);
    assert_ne!(manifest(&store), before);
    assert_ne!(store.current().unwrap().unwrap().bin, old);
    assert_eq!(store.current().unwrap().unwrap().tools, desired);
    for tool in &desired {
        assert!(store.can_reuse(tool).unwrap());
    }
    assert_eq!(
        store
            .reconcile(&desired, |_, _| panic!("normal reuse"))
            .unwrap(),
        Outcome::Unchanged
    );
    assert_eq!(fs::read(old.join("node")).unwrap(), b"1.2.3");
}

#[test]
fn forced_staging_and_late_precondition_failure_preserve_prior_inventory() {
    let repo = tempfile::tempdir().unwrap();
    let mut store = Store::open(repo.path()).unwrap();
    let desired = [tool("node"), tool("pnpm")];
    store.reconcile(&desired, populate).unwrap();
    let before = manifest(&store);
    for late in [false, true] {
        let mut staged = Vec::new();
        assert!(
            store
                .force_reconcile_checked(
                    &desired,
                    |tool, root| {
                        staged.push(tool.id.clone());
                        populate(tool, root)?;
                        if !late && tool.id == "pnpm" {
                            return Err(io::Error::other("staging failed").into());
                        }
                        Ok(())
                    },
                    || Err(io::Error::other("source changed").into())
                )
                .is_err()
        );
        assert_eq!(staged, ["node", "pnpm"]);
        assert_eq!(manifest(&store), before);
        assert!(store.is_current(&desired).unwrap());
    }
    // Fresh callbacks still undergo full tree and executable validation.
    assert!(
        store
            .force_reconcile_checked(&desired, |_, _| Ok(()), || Ok(()))
            .is_err()
    );
    assert_eq!(manifest(&store), before);
    assert!(store.can_reuse(&desired[0]).unwrap());
    assert!(store.can_reuse(&desired[1]).unwrap());
}
