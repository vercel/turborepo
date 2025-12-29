//! Integration tests for turborepo.
//!
//! These tests run turbo in isolated temp directories to provide
//! reproducible test environments without external dependencies.
//!
//! # Running the tests
//!
//! These tests are behind a feature flag:
//!
//! ```sh
//! # First, build the turbo binary
//! cargo build -p turbo
//!
//! # Run integration tests
//! cargo test -p turborepo-integration-tests --features integration-tests
//! ```
//!
//! # Architecture
//!
//! - Each test gets its own temp directory for maximum isolation
//! - The turbo binary is discovered from `target/debug/turbo`
//! - Fixtures are copied from `turborepo-tests/integration/fixtures/`
//! - Assertions use `insta` snapshots with redactions for dynamic values

// This crate is test-only, the actual code lives in tests/
