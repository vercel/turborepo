//! turborepo-config: Configuration management for Turborepo
//!
//! This crate provides the configuration system for Turborepo, handling:
//! - Environment variable configuration
//! - Configuration file reading (global/local config.json files)
//! - Configuration merging and layering
//! - Configuration validation and error reporting
//!
//! # Note on turbo.json
//!
//! This crate does NOT parse turbo.json files directly. The turbo.json parsing
//! logic lives in `turborepo-lib::turbo_json` due to circular dependency
//! constraints. Callers should use
//! `TurborepoConfigBuilder::with_turbo_json_config` to provide configuration
//! options extracted from turbo.json.

// Module declarations
mod config;
pub(crate) mod env;
mod error;
pub(crate) mod file;
pub(crate) mod override_env;

// Re-exports
pub use config::*;
pub use error::*;
