# turborepo-integration-tests

Rust-based integration tests for turborepo, replacing the prysk-based tests.

## Prerequisites

- **Rust toolchain**: For building the turbo binary and running tests
- **Git**: Required for test fixtures (turbo requires a git repository)
- **Pre-built turbo binary**: Run `cargo build -p turbo` before tests

## Running the Tests

These tests are behind a feature flag:

```sh
# First, build the turbo binary
cargo build -p turbo

# Run integration tests
cargo test -p turborepo-integration-tests --features integration-tests
```

## Architecture

### Isolation Strategy

Tests run in isolated temp directories with controlled environment variables, matching the behavior of the existing prysk-based integration tests:

- Each test gets its own temp directory via `tempfile::tempdir()`
- Fixtures are copied into the temp directory
- Git is initialized for turbo to work properly
- Environment variables are controlled for deterministic output
- Cleanup happens automatically when the test completes

### Fixtures

Fixtures are copied from `turborepo-tests/integration/fixtures/` into the temp directory at runtime. The test sets up git before running turbo commands.

### Assertions

Tests use [insta](https://insta.rs/) for snapshot testing with redactions for dynamic values:

- **Timing**: `Time: 1.23s` -> `Time: [TIME]`
- **Hashes**: `0555ce94ca234049` -> `[HASH]`

The `redact_output()` function in `common.rs` handles these redactions using compiled regexes for performance.

When snapshots change, review them with:

```sh
cargo insta review
```

## Adding a New Test

1. Create a new test file in `tests/` (e.g., `tests/run_something.rs`)
2. Use the `common` module for test setup:

```rust
#![cfg(feature = "integration-tests")]

mod common;

use common::{redact_output, TurboTestEnv};
use anyhow::Result;
use insta::assert_snapshot;

#[tokio::test]
async fn test_something() -> Result<()> {
    let env = TurboTestEnv::new().await?;

    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_success();

    // Use insta for snapshots with redaction
    assert_snapshot!(redact_output(&result.combined_output()));

    Ok(())
}
```

3. Run the test to generate initial snapshots:

```sh
cargo test -p turborepo-integration-tests --features integration-tests -- test_something
cargo insta review
```

## Migration from Prysk

This test system is being developed as a replacement for the prysk-based tests in `turborepo-tests/`. During the migration period, both systems will coexist.

The goal is feature parity:

- Same test coverage
- Same assertions (output matching with regex support)
- Same failure behavior

Benefits of the new system:

- Runs as part of `cargo test` (benefits from nextest partitioning)
- No Python dependency
- Better IDE integration
- Easier debugging with Rust tooling
- Platform matrix testing in CI (Linux, macOS, Windows)
