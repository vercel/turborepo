# Turborepo Microfrontends Configuration Architecture

## Overview

This crate provides configuration parsing for Turborepo's native microfrontends proxy. The design emphasizes **strict separation of concerns** between:

1. **Turborepo's native proxy** - Handles local development traffic routing
2. **Provider packages** (e.g., `@vercel/microfrontends`) - Handle production features, orchestration, and advanced capabilities

## Two Configuration Schemas

### 1. Turborepo Strict Schema (`turborepo_schema.rs`)

**Purpose**: Defines ONLY the configuration fields that Turborepo's native proxy needs to function.

**Supported Fields**:

- `version` - Config version for forwards compatibility
- `options.localProxyPort` - Local proxy server port (default: 3024)
- `applications[].packageName` - Package name (defaults to application key)
- `applications[].development.local` - Local dev server port/host
- `applications[].development.fallback` - Fallback URL when dev server is unavailable
- `applications[].routing` - Path routing rules for request matching

**Design Principle**: The parser ONLY deserializes fields it needs. Any extra fields in the JSON are silently ignored, making it compatible with extended schemas from providers.

### 2. Full Configuration (`configv1.rs`)

**Purpose**: Maintains backward compatibility by parsing ALL fields, including provider-specific ones.

**Additional Fields**:

- `applications[].development.task` - Task orchestration (provider concern)
- `partOf` - Child config references (Vercel feature)
- `production` - Production deployment config
- `vercel` - Vercel-specific metadata
- `assetPrefix` - Production asset handling
- `options.disableOverrides` - Vercel toolbar control

## Why This Separation?

### Problem It Solves

Previously, the full `Config` struct parsed provider-specific fields that Turborepo's proxy didn't need. This created three issues:

1. **Lock-step versioning**: Changes to Vercel's schema would break Turborepo's parser
2. **Boundary confusion**: Unclear which fields belonged to the proxy vs providers
3. **Scope creep**: Proxy code couldn't distinguish between its own concerns and provider features

### Solution: Extendable Design

```
microfrontends.json (shared config file)
       ↙        ↖

Turborepo Strict Parser         Vercel Parser
(turborepo_schema.rs)      (in @vercel/microfrontends)
    ↓                              ↓
TurborepoConfig              ExtendedConfig
(proxy routing)              (routing + orchestration)
```

Both parsers read the SAME config file. Each extracts only the fields it needs:

- **Turborepo** extracts: `version`, `options.localProxyPort`, `applications[].development.local`, `applications[].routing`, `applications[].development.fallback`
- **Vercel** extracts: Everything above PLUS `task`, `partOf`, `production`, `vercel`, etc.

## Configuration Fields Explained

### Turborepo Proxy Concerns

These fields are used by Turborepo's native proxy to route traffic:

```jsonc
{
  "applications": {
    "web": {
      "development": {
        "local": 3000, // Where to forward requests
        "fallback": "https://..." // Fallback URL if local server fails
      }
    },
    "api": {
      "routing": [
        { "paths": ["/api/*"] } // What paths route to this app
      ],
      "development": {
        "local": 3001
      }
    }
  }
}
```

### Provider Concerns

These fields are handled by provider packages, NOT by Turborepo's proxy:

```jsonc
{
  "applications": {
    "web": {
      "development": {
        "task": "dev" // Task execution handled by provider orchestration
      },
      "partOf": "web" // Child config reference (Vercel feature)
    },
    "production": {
      // Production deployment (provider concern)
      "protocol": "https",
      "host": "example.com"
    },
    "vercel": {
      // Provider-specific metadata
      "projectId": "prj_123"
    }
  }
}
```

## Public API

### `TurborepoStrictConfig`

Use this when you want ONLY Turborepo's proxy configuration:

```rust
use turborepo_microfrontends::TurborepoStrictConfig;

let config = TurborepoStrictConfig::load_from_dir(repo_root, package_dir)?;
if let Some(cfg) = config {
    let port = cfg.port("web")?;
    let fallback = cfg.fallback("web");
    let routes = cfg.routing("api")?;
}
```

### `Config`

Use this for full configuration (including provider fields):

```rust
use turborepo_microfrontends::Config;

let config = Config::load_from_dir(repo_root, package_dir)?;
// Has access to all fields, including task, production, vercel, etc.
```

### `TurborepoConfig`

Low-level direct access to the strict schema struct:

```rust
use turborepo_microfrontends::TurborepoConfig;

let config = TurborepoConfig::from_str(json_string, "path/to/config")?;
```

## For Provider Package Authors

If you're building a provider package like `@vercel/microfrontends`:

1. **Use the shared config file**: `microfrontends.json` (or `.jsonc`)
2. **Create your own parser**: Define your own schema struct that includes provider-specific fields
3. **Reuse the strict schema**: You can embed `TurborepoConfig` or reimplement the strict parsing
4. **Extend gracefully**: Only deserialize your provider-specific fields; ignore unknown fields

Example provider implementation:

```rust
use turborepo_microfrontends::TurborepoConfig;

pub struct VercelMicrofrontendsConfig {
    // Reuse Turborepo's base fields
    base: TurborepoConfig,

    // Add Vercel-specific fields
    task: Option<String>,
    partOf: Option<String>,
    production: Option<ProductionConfig>,
    vercel: Option<VercelConfig>,
}

impl VercelMicrofrontendsConfig {
    pub fn from_turborepo_config(base: TurborepoConfig, vercel_fields: Map) -> Self {
        // Combine base Turborepo config with Vercel extensions
    }
}
```

## Configuration Loading in Turborepo

When Turborepo runs:

1. **Task Setup** (`turborepo-lib/src/microfrontends.rs`):

   - Uses `Config` to parse task information
   - Determines which apps need dev tasks

2. **Proxy Startup** (`turborepo-lib/src/run/mod.rs`):

   - Re-reads the config file
   - Creates `ProxyServer` with `Config`
   - Server uses `TurborepoStrictConfig` for routing

3. **Request Routing** (`turborepo-microfrontends-proxy/src/`):
   - Router uses only proxy-relevant fields: `port`, `routing`, `fallback`
   - Never touches provider-specific fields

## Testing

Each schema has dedicated tests:

```bash
cargo test -p turborepo-microfrontends
```

**Turborepo schema tests** (`turborepo_schema.rs::test`):

- Port generation and parsing
- Routing configuration
- Root route app detection
- Fallback URL handling

**Full config tests** (`configv1.rs::test`):

- Version compatibility
- Provider-specific fields
- Task parsing
- Child config references

## Backward Compatibility

The `Config` type continues to work exactly as before, ensuring no breaking changes to existing code. New code should prefer `TurborepoStrictConfig` for clarity about which fields are being used.
