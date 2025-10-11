# Error Classification System

This document describes the error classification system in turborepo for consistent error handling across the codebase.

## Overview

The error classification system provides a standardized way to categorize errors based on their nature. This helps with:

- **Exit code determination**: Consistent exit codes based on error type
- **Error reporting and metrics**: Tracking error categories for telemetry
- **User-facing error messages**: Providing appropriate guidance based on error type
- **Debugging and troubleshooting**: Quick identification of error sources

## Error Classifications

The system defines the following error classifications:

| Classification     | Description                                  | Exit Code | Retryable |
| ------------------ | -------------------------------------------- | --------- | --------- |
| `Configuration`    | Invalid config, missing config files         | 1         | No        |
| `Authentication`   | Auth and authorization errors                | 1         | No        |
| `Network`          | Timeouts, connection refused, DNS failures   | 1         | Yes       |
| `FileSystem`       | File not found, permission denied, disk full | 1         | No        |
| `ProcessExecution` | Spawn failures, non-zero exit codes          | 1         | No        |
| `UserInput`        | Invalid user input or arguments              | 2         | No        |
| `Internal`         | Internal logic errors or bugs                | 100       | No        |
| `Dependency`       | Errors from external dependencies            | 1         | No        |
| `Cache`            | Cache-related errors                         | 1         | Yes       |
| `TaskExecution`    | Task graph and execution errors              | 1         | No        |
| `Daemon`           | Daemon-related errors                        | 1         | Yes       |
| `Environment`      | Missing env vars, platform issues            | 1         | No        |
| `Parsing`          | JSON, JSONC, package.json parsing errors     | 1         | No        |
| `Proxy`            | Proxy and networking errors (microfrontends) | 1         | Yes       |

## Usage

### Implementing the `Classify` trait

To add classification to your error type, implement the `Classify` trait:

```rust
use turborepo_errors::{Classify, ErrorClassification};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl Classify for MyError {
    fn classify(&self) -> ErrorClassification {
        match self {
            MyError::Config(_) => ErrorClassification::Configuration,
            MyError::Network(_) => ErrorClassification::Network,
            MyError::Internal(_) => ErrorClassification::Internal,
        }
    }
}
```

### Using error classification

Once implemented, you can use the classification for various purposes:

```rust
use turborepo_errors::Classify;

fn handle_error(error: &dyn Classify) {
    let classification = error.classify();

    // Get suggested exit code
    let exit_code = classification.exit_code();

    // Check if error is retryable
    if classification.is_retryable() {
        println!("This error may be transient, consider retrying");
    }

    // Check if it's a user error
    if classification.is_user_error() {
        println!("Please check your configuration or input");
    }

    // Check if it's an internal bug
    if classification.is_internal_error() {
        println!("This is an internal error, please report it");
    }

    // Get category name for logging
    println!("Error category: {}", classification.category_name());
}
```

### Best Practices

1. **Be specific**: Choose the most specific classification that fits your error
2. **User errors**: Use `Configuration` or `UserInput` for errors that users can fix
3. **Internal errors**: Use `Internal` only for bugs or unexpected states
4. **Network errors**: Use `Network` for connectivity issues, `Proxy` for proxy-specific issues
5. **Consistent mapping**: For transparent error wrappers, classify based on the underlying error type

### Examples

#### Configuration Error

```rust
Error::NoTurboJSON => ErrorClassification::Configuration
```

#### Network Error

```rust
Error::Reqwest(_) => ErrorClassification::Network
```

#### Internal Error

```rust
Error::InternalErrors(_) => ErrorClassification::Internal
```

#### Task Execution Error

```rust
TaskErrorCause::Exit { .. } => ErrorClassification::TaskExecution
```

## Classification Guidelines

### Configuration vs UserInput

- **Configuration**: Issues with config files (turbo.json, package.json)
- **UserInput**: Issues with command-line arguments or flags

### Network vs Proxy

- **Network**: General connectivity issues, API errors
- **Proxy**: Specific to the microfrontends proxy (port binding, app unreachable)

### Internal vs Others

- **Internal**: Logic bugs, panics, unexpected states
- **Others**: Expected error conditions that users can understand and fix

### FileSystem vs Environment

- **FileSystem**: File operations (read, write, permissions)
- **Environment**: Missing or invalid environment variables, platform issues

## Adding New Classifications

When adding a new classification:

1. Add it to the `ErrorClassification` enum in `classification.rs`
2. Update all match statements in the implementation
3. Add appropriate tests
4. Update this documentation

## Testing

The classification module includes comprehensive tests:

```bash
cargo test -p turborepo-errors
```

Tests verify:

- Exit codes are in valid range (1-255)
- Retryable classifications are correctly identified
- User error classifications are correctly identified
- Internal error classifications are correctly identified
- Category names are properly formatted
- Display implementation works correctly
