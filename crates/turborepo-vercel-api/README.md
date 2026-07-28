# turborepo-vercel-api

## Purpose

Type definitions for the Vercel API. Shared between the API client (`turborepo-api-client`) and mock server (`turborepo-vercel-api-mock`).

## Architecture

```
turborepo-vercel-api
    └── API types
        ├── User, Team, Membership
        ├── CachingStatus, ArtifactResponse
        ├── VerificationResponse (SSO)
        ├── telemetry/ - Telemetry event types
        └── token/ - Token metadata types
```

## Notes

Pure type definitions with serialization support. No logic - just the contract between client and server.
