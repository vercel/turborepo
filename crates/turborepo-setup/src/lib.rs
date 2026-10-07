//! Internal setup adapters. Shared OS/architecture values live in
//! `turborepo-platform`; vendor artifact policy lives in each adapter.

pub mod lock;
pub mod node;
pub mod node_discovery;
pub mod node_metadata;
pub mod node_provision;
pub mod package_manager;
pub mod pnpm_provision;
pub mod registry_metadata;
pub mod version_request;
pub mod writer_storage;

pub use node::{NodeArtifact, NodeArtifactError};
pub use node_discovery::{
    NodeDiscoveryError, NodeRelease, NodeRequirements, NodeSource, ResolvedNode,
};
pub use version_request::{VersionRequest, VersionRequestError};
