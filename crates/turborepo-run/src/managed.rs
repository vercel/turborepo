//! Scoped run input: one selected snapshot and its separately authorized tools.

use std::sync::Arc;

use turborepo_setup::execution_identity::ExecutionSnapshot;

use crate::{EnvironmentVariableMap, ManagedEnvironment};

/// Construct only after external setup authorization. The pure snapshot is not
/// authority; the environment owner must freshly revalidate at actual use.
pub struct ManagedRun {
    snapshot: ExecutionSnapshot,
    environment: Arc<dyn ManagedEnvironment>,
}

impl ManagedRun {
    pub fn new(
        snapshot: ExecutionSnapshot,
        environment: Arc<dyn ManagedEnvironment>,
    ) -> Result<Self, String> {
        if snapshot.fingerprint() != environment.cache_identity() {
            return Err(
                "managed snapshot and authorized environment disagree; run turbo setup".into(),
            );
        }
        Ok(Self {
            snapshot,
            environment,
        })
    }

    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }
    pub fn revalidate(&self) -> Result<EnvironmentVariableMap, String> {
        self.environment.revalidate()
    }
}

impl ManagedEnvironment for ManagedRun {
    fn revalidate(&self) -> Result<EnvironmentVariableMap, String> {
        ManagedRun::revalidate(self)
    }
    fn cache_identity(&self) -> &str {
        self.snapshot.fingerprint()
    }
}
