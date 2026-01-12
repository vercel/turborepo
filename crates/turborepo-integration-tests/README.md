# turborepo-integration-tests

Rust-based test setup infrastructure for turborepo integration tests.

## Overview

This crate provides a Rust library and CLI binary (`turbo-test-setup`) for setting up integration test environments. The test setup is used by the prysk-based integration tests to ensure consistent, cross-platform test environments.

## Components

### Library (`src/lib.rs`)

The library exports:

- `TurboTestEnv` - A test environment struct for running turbo commands in isolated temp directories
- `redact_output()` - Helper for redacting dynamic values (hashes, timing) from output
- `copy_dir_recursive()` - Utility for copying fixture directories
- Path helpers (`turbo_binary_path()`, `fixtures_path()`)

### CLI Binary (`turbo-test-setup`)

The binary can be called from shell scripts to set up test environments:

```sh
# Initialize a test environment (outputs shell commands to eval)
eval "$(turbo-test-setup init basic_monorepo)"

# With custom package manager
eval "$(turbo-test-setup init basic_monorepo --package-manager npm@10.5.0)"

# Skip dependency installation
eval "$(turbo-test-setup init basic_monorepo --no-install)"
```

The binary outputs shell variable assignments that set up the environment:

- `TURBO` - Path to the turbo binary
- `TURBO_TELEMETRY_MESSAGE_DISABLED=1`
- `TURBO_GLOBAL_WARNING_DISABLED=1`
- `TURBO_PRINT_VERSION_DISABLED=1`
- `PATH` - Updated to include corepack shim directory

## Building

```sh
# Build the setup binary
cargo build -p turborepo-integration-tests

# The binary will be at target/debug/turbo-test-setup
```

## Usage with Prysk Tests

The prysk integration tests in `turborepo-tests/integration/tests/` will automatically use this binary if it's available:

```bash
# In setup_integration_test.sh
if [[ -x "$TURBO_TEST_SETUP" ]]; then
  eval "$($TURBO_TEST_SETUP init $FIXTURE_NAME $SETUP_ARGS)"
else
  # Fall back to shell-based setup
  ...
fi
```

## Benefits

- **Cross-platform consistency**: Handles line ending normalization, path handling
- **Reproducible environments**: Uses corepack for package manager version pinning
- **Faster setup**: Rust binary can be faster than shell scripts, especially on Windows
- **Shared infrastructure**: Same setup code used by both Rust and prysk tests
