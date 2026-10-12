//! Locked Yarn 1 from the classic `yarn` registry package, launched by the
//! selected managed Node. No Corepack, install hooks, or dependency
//! installation.

use turborepo_tool_install::{Store, Tool};

pub use crate::registry_provision::{
    Error, PreparedRegistry as PreparedYarn, RegistryTransport as YarnTransport,
};
use crate::{
    lock::{Lock, Platform},
    node_provision::NodePlan,
    package_manager::{CorepackIntegrity, Declaration, Manager},
    registry_provision::RegistryPlan,
};

pub struct YarnPlan(RegistryPlan);
impl YarnPlan {
    /// Bind the applicable `packageManager`/devEngines integrity.
    pub fn from_declaration(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        declaration: &Declaration,
    ) -> Result<Self, Error> {
        RegistryPlan::from_declaration(lock, platform, node, declaration, Manager::Yarn).map(Self)
    }

    pub fn from_lock(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        authored: Option<&CorepackIntegrity>,
    ) -> Result<Self, Error> {
        RegistryPlan::from_lock(lock, platform, node, authored, Manager::Yarn).map(Self)
    }

    pub fn inventory_tool(&self) -> &Tool {
        self.0.inventory_tool()
    }

    /// Prepare only; reconcile the same complete desired inventory afterward.
    pub async fn prepare_if_needed(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &YarnTransport,
    ) -> Result<Option<PreparedYarn>, Error> {
        self.0.prepare_if_needed(store, desired, transport).await
    }

    /// Select fresh verification without changing the locked identity.
    pub async fn prepare(
        &self,
        store: &Store,
        desired: &[Tool],
        transport: &YarnTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedYarn>, Error> {
        self.0.prepare(store, desired, transport, preparation).await
    }
}

#[cfg(all(test, unix))]
mod tests;
