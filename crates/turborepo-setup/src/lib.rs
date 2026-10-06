//! Internal setup adapters. Shared OS/architecture values live in
//! `turborepo-platform`; vendor artifact policy lives in each adapter.

pub mod lock;
pub mod node;
pub mod node_discovery;
pub mod package_manager;
pub mod version_request;

pub use node::{NodeArtifact, NodeArtifactError};
pub use node_discovery::{
    NodeDiscoveryError, NodeRelease, NodeRequirements, NodeSource, ResolvedNode,
};
pub use version_request::{VersionRequest, VersionRequestError};
