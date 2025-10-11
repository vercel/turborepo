# Error Context Improvements

This document summarizes the error context improvements made to the turborepo codebase, following the pattern of using `.map_err()` to add descriptive context to errors.

## Overview

The error context improvement strategy focuses on:

1. Adding descriptive error messages that include relevant context
2. Using structured error types with the `thiserror` crate
3. Implementing the `Classify` trait for error categorization
4. Providing actionable information to users when errors occur

## Pattern Example

```rust
// Good: Adding context with .map_err()
Router::new(&config)
    .map_err(|e| ProxyError::Config(format!(
        "Failed to build router from config: {}",
        e
    )))?;

// Even better: Including file paths or other context
Router::new(&config)
    .map_err(|e| ProxyError::Config(format!(
        "Failed to build router from config at {}: {}",
        config_path, e
    )))?;
```

## Improvements Made

### 1. Proxy Module (`crates/turborepo-microfrontends-proxy/src/proxy.rs`)

#### Router Creation Error (line 60-61)

```rust
let router = Router::new(&config)
    .map_err(|e| ProxyError::Config(format!("Failed to build router: {}", e)))?;
```

**Context Added**: Indicates that router building failed, includes underlying error.

#### Port Binding Error (line 100-105)

```rust
let listener = TcpListener::bind(addr)
    .await
    .map_err(|e| ProxyError::BindError {
        port: self.port,
        source: e,
    })?;
```

**Context Added**: Structured error includes the port number that failed to bind.

#### HTTP Error Conversion (line 386)

```rust
.map_err(ProxyError::Http)?;
```

**Context Added**: Converts generic HTTP errors to typed ProxyError variants.

### 2. Router Module (`crates/turborepo-microfrontends-proxy/src/router.rs`)

#### Port Configuration Error (lines 59-64)

**Before**:

```rust
.ok_or_else(|| format!("No port configured for application '{}'", app_name))?;
```

**After**:

```rust
.ok_or_else(|| {
    format!(
        "No port configured for application '{}'. Check your configuration file.",
        app_name
    )
})?;
```

**Context Added**: Includes actionable advice to check the configuration file.

#### Routing Pattern Parsing Error (lines 72-77)

**Before**:

```rust
patterns.push(PathPattern::parse(path)?);
```

**After**:

```rust
patterns.push(PathPattern::parse(path).map_err(|e| {
    format!(
        "Invalid routing pattern '{}' for application '{}': {}",
        path, app_name, e
    )
})?);
```

**Context Added**: Includes the invalid pattern, the application name, and the underlying error.

#### Default Application Error (lines 91-93)

**Before**:

```rust
let default_app = default_app.ok_or_else(|| {
    "No default application found (application without routing configuration)".to_string()
})?;
```

**After**:

```rust
let default_app = default_app.ok_or_else(|| {
    "No default application found. At least one application without routing configuration is required.".to_string()
})?;
```

**Context Added**: Clarifies what's required to fix the issue.

#### Empty Pattern Error (line 206)

**Before**:

```rust
return Err("Pattern cannot be empty".to_string());
```

**After**:

```rust
return Err("Routing pattern cannot be empty. Provide a valid path pattern like '/' or '/docs/:path*'".to_string());
```

**Context Added**: Includes examples of valid patterns.

#### Empty Parameter Name Error (lines 227-229)

**New Error Added**:

```rust
if param_name.is_empty() {
    return Err("Parameter name cannot be empty after ':'. Use a format like ':id' or ':path*'".to_string());
}
```

**Context Added**: Catches a new error case and provides examples of correct usage.

### 3. Error Type Definitions (`crates/turborepo-microfrontends-proxy/src/error.rs`)

#### Structured Error Enum with thiserror

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("Failed to bind to port {port}: {source}")]
    BindError { port: u16, source: std::io::Error },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Failed to connect to application '{app}' on port {port}")]
    AppUnreachable { app: String, port: u16 },

    // ... other variants
}
```

#### Error Classification

```rust
impl Classify for ProxyError {
    fn classify(&self) -> ErrorClassification {
        match self {
            ProxyError::BindError { .. } => ErrorClassification::Network,
            ProxyError::Config(_) => ErrorClassification::Configuration,
            ProxyError::AppUnreachable { .. } => ErrorClassification::Proxy,
            // ... other classifications
        }
    }
}
```

**Benefit**: Allows programmatic error handling and consistent error reporting.

### 4. User-Facing Error Pages

The `ErrorPage` struct generates beautiful HTML error pages with:

- The request path that failed
- The expected application and port
- The specific error message
- Troubleshooting steps
- Command suggestions (e.g., `turbo run {app}#dev`)

## Error Context Best Practices

### ✅ DO:

1. **Include relevant context**: app names, file paths, port numbers, etc.
2. **Provide actionable advice**: "Check your configuration file", "Verify port X is not in use"
3. **Include examples**: Show users what a valid input looks like
4. **Use structured errors**: Create specific error variants with typed fields
5. **Chain errors**: Preserve the underlying error while adding context

### ❌ DON'T:

1. **Use generic messages**: "An error occurred" is not helpful
2. **Hide the underlying error**: Always include the source error
3. **Assume context**: The user may not know which file or configuration is being processed
4. **Use technical jargon**: Keep messages accessible to all users

## Testing Error Messages

All error improvements include tests that verify:

1. The error is triggered correctly
2. The error message contains expected context
3. Examples in error messages are valid

Example test:

```rust
#[test]
fn test_pattern_parse_errors() {
    let err = PathPattern::parse("").unwrap_err();
    assert!(err.contains("cannot be empty"));

    let err = PathPattern::parse("/api/:").unwrap_err();
    assert!(err.contains("Parameter name cannot be empty"));
}
```

## Future Improvements

Consider these enhancements for future PRs:

1. Add more specific error variants to `ProxyError` instead of using `String`
2. Include file paths in configuration errors
3. Add error codes for programmatic error handling
4. Implement error recovery suggestions
5. Add telemetry for error tracking (while respecting privacy)

## Related Documentation

- Error classification system: `crates/turborepo-errors/src/classification.rs`
- Error classification example: `crates/turborepo-errors/examples/classification_example.rs`
- Contributing guidelines: `CONTRIBUTING.md`
