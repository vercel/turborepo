//! Bridge between ephemeral CI setup and committed-lock authorization. No
//! synthetic lock or ephemeral execution record is created by publication.
use super::*;
use crate::commands::managed::{self, ManagedTools};

#[test]
fn missing_lock_publication_succeeds_and_revokes_previous_locked_authority() {
    let f = Fixture::new();
    let upstream = Upstream::new(&f, false, None);
    f.run(&upstream, FROZEN).unwrap();
    let lock = fs::read(f.owned.root().join("turbo.lock")).unwrap();
    let record = fs::read_dir(f.owned.home().join("repos"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("record.json");
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    let sources = f.sources().unwrap();
    let writer = sources.guard().unwrap();
    sources.check_guard(&writer).unwrap();
    let user_store = turborepo_tool_install::UserStore::at(f.owned.home(), f.owned.root()).unwrap();
    let trust = super::trust::TrustStore::new(&user_store, f.owned.root()).unwrap();
    let env = trust.recorded_env().unwrap().unwrap();
    trust.publish(&env, None).unwrap();
    trust.publish(&env, None).unwrap(); // Repeat does not create a fake record/lock.
    assert!(!record.exists());
    assert!(!f.owned.root().join("turbo.lock").exists());
    // The environment stays installed in the store; only the authority goes.
    assert!(f.owned.home().join("envs").join(env.as_str()).is_dir());
    assert!(
        ManagedTools::inspect(
            f.owned.root(),
            &f.owned.root().join("turbo.json"),
            true,
            Some(f.owned.home()),
            || panic!("missing-lock repo must not probe the host")
        )
        .unwrap()
        .is_none()
    );
    drop(writer);
    fs::write(f.owned.root().join("turbo.lock"), lock).unwrap();
    assert!(
        ManagedTools::inspect(
            f.owned.root(),
            &f.owned.root().join("turbo.json"),
            true,
            Some(f.owned.home()),
            managed::context
        )
        .is_err()
    );
    assert_eq!(upstream.hits(), 3);
    f.no_execution();
}
