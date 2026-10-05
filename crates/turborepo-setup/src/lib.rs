//! Internal setup adapters. Shared OS/architecture values live in
//! `turborepo-platform`; vendor artifact policy lives in each adapter.

pub mod node;
pub mod version_request;

pub use node::{NodeArtifact, NodeArtifactError};
pub use version_request::{VersionRequest, VersionRequestError};
