//! turborepo-config: Configuration management for Turborepo
//!
//! This crate provides the configuration system for Turborepo, handling:
//! - Configuration file parsing (turbo.json)
//! - Environment variable configuration
//! - Configuration merging and layering
//! - Configuration validation and error reporting

// Module declarations
mod config;
pub(crate) mod env;
mod error;
pub(crate) mod file;
pub(crate) mod override_env;
pub(crate) mod turbo_json;

// Re-exports
pub use config::*;
pub use error::*;
