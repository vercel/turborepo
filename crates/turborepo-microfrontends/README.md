# Turborepo Microfrontends Configuration Parser

This crate provides parsing and validation for `microfrontends.json` configuration files used by both Turborepo's local development proxy and Vercel's production microfrontends integration.

Note: We mention Vercel since this is the only provider with integration today. We would be delighted to enable integration for more providers. If you are interested in doing so, please reach out to the Turborepo core team.

## Purpose

This crate parses the minimal amount of information that Turborepo needs to correctly invoke a local microfrontends proxy. By parsing only what's needed, this crate can remain independent of the `@vercel/microfrontends` package while still supporting the same configuration format.

## Key Features

## Configuration Schema

The crate parses `microfrontends.json` files with the following structure:

```json
{
  "version": "1",
  "options": {
    "localProxyPort": 3024
  },
  "applications": {
    "app-name": {
      "packageName": "optional-package-name",
      "development": {
        "local": { "port": 3000 },
        "task": "dev"
      },
      "routing": [
        {
          "paths": ["/path", "/path/:slug*"],
          "group": "optional-group-name"
        }
      ]
    }
  }
}
```

## What This Crate Parses

### Used by Turborepo

- ✅ `version`: Configuration version
- ✅ `options.localProxyPort`: Port for local proxy server
- ✅ `applications`: Application configurations
- ✅ `applications[].packageName`: Package name mapping
- ✅ `applications[].development.local`: Local development port
- ✅ `applications[].development.task`: Development task name
- ✅ `applications[].routing`: Path routing configuration

## Usage

```rust
use turborepo_microfrontends::{Config, PathGroup};
use turbopath::AbsoluteSystemPath;

// Load configuration from a file
let config_path = AbsoluteSystemPath::new("/path/to/microfrontends.json")?;
let config = Config::load(config_path)?;

if let Some(config) = config {
    // Access development tasks
    for task in config.development_tasks() {
        println!("App: {}, Task: {:?}", task.application_name, task.task);
    }

    // Get local proxy port
    if let Some(port) = config.local_proxy_port() {
        println!("Proxy port: {}", port);
    }

    // Get routing configuration
    if let Some(routing) = config.routing("app-name") {
        for path_group in routing {
            println!("Paths: {:?}", path_group.paths);
        }
    }
}
```

## Configuration Files

### Default Names

- `microfrontends.json` (primary)
- `microfrontends.jsonc` (alternative, supports comments)

### Default Package

## Design Principles

1. **Permissive Parsing**: Accept all valid Vercel configurations
2. **Graceful Degradation**: Ignore production fields without erroring
3. **Forward Compatibility**: New Vercel-only fields won't break Turborepo
4. **Minimal Dependencies**: Parse only what Turborepo needs
5. **Clear Separation**: Production features stay in `@vercel/microfrontends`

## Coexistence Model

This crate uses a coexistence model. It looks for the `@vercel/microfrontends` package in the workspace to determine proxy selection.

```
Same monorepo can have:

Package A (has @vercel/microfrontends)
  └── Uses Vercel proxy with full production features

Package B (no @vercel/microfrontends)
  └── Uses Turborepo proxy for local dev only

Both packages read the same microfrontends.json format!
```

## Testing

```bash
cargo test
```

The test suite includes:

- Version validation
- Configuration parsing (with and without version)
- Child configuration handling
- Package name mapping
- Port generation
- Directory loading
- Error handling

## Future Work

This is phase 1 of the Turborepo microfrontends feature. Future phases will include:

- **Phase 1**: Proxy implementation (`turborepo-microfrontends-proxy` crate)
- **Phase 2**: Documentation and examples
