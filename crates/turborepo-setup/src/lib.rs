//! Internal setup adapters. Shared OS/architecture values live in
//! `turborepo-platform`; vendor artifact policy lives in each adapter.

pub mod activation;
pub mod artifact_provision;
pub mod bundled_npm;
pub mod cargo_prefetch;
pub mod ephemeral_reconcile;
pub mod execution_identity;
pub mod go_discovery;
pub mod go_provision;
pub mod go_resolution;
pub mod js_resolution;
pub mod lifecycle;
pub mod lock;
pub mod native_baseline;
pub mod native_resolution;
pub mod node;
pub mod node_discovery;
pub mod node_metadata;
pub mod node_provision;
pub mod node_resolution;
pub mod npm_provision;
pub mod package_manager;
pub mod pep440;
pub mod python_discovery;
pub mod python_provision;
pub mod python_resolution;
mod registry_provision;
pub mod pnpm_provision {
    pub use crate::registry_provision::{
        Error, PnpmPlan, PreparedRegistry as PreparedPnpm, RegistryTransport as PnpmTransport,
    };
}
pub mod registry_metadata;
pub mod registry_resolution;
pub mod rust_discovery;
pub mod rust_metadata;
pub mod rust_provision;
pub mod rust_resolution;
pub mod rustup;
pub mod rustup_provision;
pub mod source_policy;
pub mod uv;
pub mod uv_discovery;
pub mod uv_metadata;
pub mod uv_provision;
pub mod uv_resolution;
pub mod uv_sync;
pub mod version_managers;
pub mod version_request;
pub mod writer_storage;

/// Locked artifact preparation policy; never changes resolution or health
/// evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preparation {
    IfNeeded,
    Fresh,
}

pub use node::{NodeArtifact, NodeArtifactError};
pub use node_discovery::{
    NodeDiscoveryError, NodeRelease, NodeRequirements, NodeSource, ResolvedNode,
};
#[cfg(feature = "test-support")]
pub use source_policy::test_support;
pub use version_request::{VersionRequest, VersionRequestError};
