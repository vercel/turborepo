use super::*;

#[test]
fn resolution_wait_preserves_actual_absent_lock_and_source_expectations() {
    for real_lock in [false, true] {
        let repo = root(Some("24.x"), json!({}));
        let p = repo.path();
        let mut store = Store::open(p).unwrap();
        let lock = seed_ephemeral(p, &mut store);
        fs::write(p.join(".nvmrc"), "^24.0.0").unwrap();
        let snapshot = Snapshot::capture(p).unwrap();
        let before = manifest(p);
        let path = p.to_owned();
        let world = World::new(node_routes(&["24.1.0"]), move |request| {
            if request == "/dist/index.json" {
                if real_lock {
                    save(&path, &lock);
                } else {
                    fs::write(path.join(".nvmrc"), "24.x").unwrap();
                }
            }
        });
        assert!(staged(&snapshot, &store, &world).is_err());
        assert_eq!(manifest(p), before);
        assert_eq!(p.join("turbo.lock").exists(), real_lock);
    }
}
