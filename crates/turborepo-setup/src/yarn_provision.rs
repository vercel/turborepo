//! Locked Yarn on the selected managed Node: Yarn 1 from the classic `yarn`
//! package, Yarn 2+ from the `@yarnpkg/cli-dist` bundle Corepack also uses.
//! No Corepack, install hooks, `yarnPath` probes, or dependency installation.

use turborepo_tool_install::{SetupSession, Tool};

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
    /// Bind the applicable `packageManager`/devEngines integrity. Modern Yarn
    /// pins are verified against the bundle bytes before every preparation.
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
        store: &SetupSession<'_>,
        desired: &[Tool],
        transport: &YarnTransport,
    ) -> Result<Option<PreparedYarn>, Error> {
        self.0.prepare_if_needed(store, desired, transport).await
    }

    /// Select fresh verification without changing the locked identity.
    pub async fn prepare(
        &self,
        store: &SetupSession<'_>,
        desired: &[Tool],
        transport: &YarnTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedYarn>, Error> {
        self.0.prepare(store, desired, transport, preparation).await
    }
}

#[cfg(all(test, unix))]
mod tests;
