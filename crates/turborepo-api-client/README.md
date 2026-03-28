# turborepo-api-client

## Purpose

HTTP client for interacting with the Remote Cache API. Handles authentication, artifact upload/download, and telemetry endpoints. By default configured for the Vercel API.

## Architecture

```
turborepo-api-client
    ├── Client trait - API abstraction
    │   ├── Authentication (tokens, SSO)
    │   ├── Cache operations (get/put artifacts)
    │   ├── Team/user info
    │   └── Telemetry
    ├── analytics/ - Cache usage analytics
    └── retry/ - Request retry logic
```

Uses `reqwest` for HTTP with automatic retries for transient failures.

## Notes

The `Client` trait allows for alternative API implementations beyond Vercel. The mock server in `turborepo-vercel-api-mock` implements the same interface for testing.
