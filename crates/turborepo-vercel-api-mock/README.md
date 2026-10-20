# turborepo-vercel-api-mock

## Purpose

Mock server implementation for the Vercel API. Used for testing without hitting real endpoints.

## Architecture

```
turborepo-vercel-api-mock
    └── Axum HTTP server
        ├── /v2/user - User info
        ├── /v2/teams - Team listing
        ├── /v8/artifacts/* - Cache operations
        ├── /v8/artifacts/events - Analytics
        └── /api/telemetry - Telemetry
```

Stores artifacts in a temporary directory during tests.

## Notes

Uses predefined constants for expected tokens and user data. Primarily used by integration tests to verify API client behavior without network dependencies.
