//! Independently locked Unix npm override. Node must export only node when npm
//! owns npm/npx. Matching native pins use `bundled_npm`: Node remains the sole
//! resource/executable owner, with a separate npm resolution identity.

use turborepo_tool_install::{SetupSession, Tool};

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

    /// Validate the exact native pin and every applicable authored constraint.
    /// Bundled npm has no separate inventory owner or registry preparation.
    pub fn from_declaration(
        lock: &Lock,
        platform: Platform,
        node: &NodePlan,
        declaration: &crate::package_manager::Declaration,
    ) -> Result<Option<Self>, Error> {
        let tool = lock.tools().get("npm").ok_or(Error::InvalidLock)?;
        if declaration.manager != Manager::Npm
            || crate::js_resolution::exact_manager(declaration).map_err(|_| Error::InvalidLock)?
                != tool.version
        {
            return Err(Error::InvalidLock);
        }
        if matches!(tool.installation, crate::lock::Installation::Bundled { .. }) {
            // Lock validation binds npm's version and exports to Node. Do not
            // interpret an authored registry digest as evidence for Node bytes.
            if declaration
                .package_manager
                .iter()
                .chain(&declaration.dev_engines)
                .any(|request| request.integrity.is_some())
                || NodePlan::from_lock(lock, platform)
                    .map_err(|_| Error::NodeBinding)?
                    .inventory_tool()
                    != node.inventory_tool()
            {
                return Err(Error::InvalidLock);
            }
            return Ok(None);
        }
        // Reject malformed portable variants and contradictory authored SHA-256
        // before selecting the host artifact, inventory reuse or any transport.
        crate::js_resolution::validate_registry(tool, declaration)
            .map_err(|_| Error::InvalidLock)?;
        let plan = Self::from_lock(lock, platform, node, None)?;
        let version = semver::Version::parse(&tool.version).map_err(|_| Error::InvalidLock)?;
        let authored =
            declaration.locked_integrity(&version, &plan.inventory_tool().artifact_sha256)?;
        Self::from_lock(lock, platform, node, authored.as_ref()).map(Some)
    }

    pub fn inventory_tool(&self) -> &Tool {
        self.0.inventory_tool()
    }

    pub fn cache_available(&self, transport: &NpmTransport) -> bool {
        self.0.cache_available(transport)
    }

    /// Force verifies fresh bytes without changing the selected resolution.
    pub async fn prepare(
        &self,
        store: &SetupSession<'_>,
        desired: &[Tool],
        transport: &NpmTransport,
        preparation: crate::Preparation,
    ) -> Result<Option<PreparedNpm>, Error> {
        self.0.prepare(store, desired, transport, preparation).await
    }

    /// Hold the Store guard through preparation and one reconciliation of this
    /// SAME complete cohort, including Node even when it is not installed yet.
    pub async fn prepare_if_needed(
        &self,
        store: &SetupSession<'_>,
        desired: &[Tool],
        transport: &NpmTransport,
    ) -> Result<Option<PreparedNpm>, Error> {
        self.0.prepare_if_needed(store, desired, transport).await
    }
}

#[cfg(all(test, unix))]
mod tests;
