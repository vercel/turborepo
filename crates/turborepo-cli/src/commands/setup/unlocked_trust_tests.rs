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
    let record = fs::read_dir(f.owned.home().join("turborepo/setup-roots"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let manifest = f.manifest().unwrap();
    fs::remove_file(f.owned.root().join("turbo.lock")).unwrap();
    let sources = turborepo_setup::lock::Snapshot::capture(f.owned.root()).unwrap();
    let writer = sources.guard().unwrap();
    let store = turborepo_tool_install::Store::open(f.owned.root()).unwrap();
    sources.check_guard(&writer).unwrap();
    let trust = super::trust::TrustStore::new(f.owned.home(), f.owned.root()).unwrap();
    trust.publish(&store).unwrap();
    trust.publish(&store).unwrap(); // Repeat does not create a fake record/lock.
    assert!(!record.exists());
    assert!(!f.owned.root().join("turbo.lock").exists());
    assert_eq!(f.manifest().unwrap(), manifest);
    assert!(
        ManagedTools::inspect(
            f.owned.root(),
            &f.owned.root().join("turbo.json"),
            f.owned.home(),
            || panic!("missing-lock repo must not probe the host")
        )
        .unwrap()
        .is_none()
    );
    drop(store);
    drop(writer);
    fs::write(f.owned.root().join("turbo.lock"), lock).unwrap();
    assert!(
        ManagedTools::inspect(
            f.owned.root(),
            &f.owned.root().join("turbo.json"),
            f.owned.home(),
            managed::context
        )
        .is_err()
    );
    assert_eq!(upstream.hits(), 3);
    f.no_execution();
}
