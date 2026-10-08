//! Independently locked Unix npm override. Node must export only node when npm
//! owns npm/npx. Matching Node-bundled selection belongs to TURBO-6326, not
//! here.

use turborepo_tool_install::{Store, Tool};

pub use crate::registry_provision::{
    Error, PreparedRegistry as PreparedNpm, RegistryTransport as NpmTransport,
};
use crate::{
    lock::{Lock, Platform},
    node_provision::NodePlan,
    package_manager::{CorepackIntegrity, Manager},
    registry_provision::RegistryPlan,
};

pub struct NpmPlan(RegistryPlan);
impl NpmPlan {
    /// Consume a validated, independently pinned npm selection. Never resolve
    /// or silently substitute Node-bundled npm for this artifact.
    pub fn from_lock(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        authored: Option<&CorepackIntegrity>,
    ) -> Result<Self, Error> {
        RegistryPlan::from_lock(lock, platform, node, authored, Manager::Npm).map(Self)
    }

    pub fn inventory_tool(&self) -> &Tool {
        self.0.inventory_tool()
    }

    /// Hold the Store guard through preparation and one reconciliation of this
    /// SAME complete cohort, including Node even when it is not installed yet.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &NpmTransport,
    ) -> Result<Option<PreparedNpm>, Error> {
        self.0.prepare_if_needed(store, desired, transport).await
    }
}

#[cfg(all(test, unix))]
mod tests;
