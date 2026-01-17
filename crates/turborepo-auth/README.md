# turborepo-auth

## Purpose

Authentication with the Vercel API. Handles login flows, SSO verification, token storage, and token refresh.

## Architecture

```
turborepo-auth
    ├── Login flow
    │   ├── Browser-based OAuth
    │   └── Local callback server
    ├── SSO verification
    ├── Token storage
    │   ├── ~/.turbo/config.json (turbo tokens)
    │   └── ~/.config/com.vercel.cli/auth.json (Vercel CLI tokens)
    └── Token refresh (OAuth)
```

Supports reading tokens from the Vercel CLI if present, allowing shared authentication.

## Notes

The login flow opens a browser for OAuth authentication and runs a temporary local server to receive the callback. Tokens are stored locally and can be refreshed automatically when expired.
