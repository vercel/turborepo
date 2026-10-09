//! Internal setup adapters. Shared OS/architecture values live in
//! `turborepo-platform`; vendor artifact policy lives in each adapter.

pub mod activation;
pub mod bundled_npm;
pub mod execution_identity;
pub mod js_resolution;
pub mod lock;
pub mod node;
pub mod node_discovery;
pub mod node_metadata;
pub mod node_provision;
pub mod node_resolution;
pub mod npm_provision;
pub mod package_manager;
mod registry_provision;
pub mod pnpm_provision {
    pub use crate::registry_provision::{
        Error, PnpmPlan, PreparedRegistry as PreparedPnpm, RegistryTransport as PnpmTransport,
    };
}
pub mod registry_metadata;
pub mod registry_resolution;
pub mod source_policy;
pub mod version_request;
pub mod writer_storage;

pub use node::{NodeArtifact, NodeArtifactError};
pub use node_discovery::{
    NodeDiscoveryError, NodeRelease, NodeRequirements, NodeSource, ResolvedNode,
};
#[cfg(feature = "test-support")]
pub use source_policy::test_support;
pub use version_request::{VersionRequest, VersionRequestError};
