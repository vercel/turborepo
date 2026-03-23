# turborepo-auth

## Purpose

Authentication with the Vercel API. Handles login flows, SSO verification, token storage, and token refresh.

## Architecture

```
turborepo-auth
    ├── Login flow
    │   ├── Device Authorization Grant (RFC 8628)
    │   └── Token refresh (OAuth)
    ├── SSO verification
    │   ├── Token introspection (RFC 7662)
    │   └── Localhost redirect for SSO callback
    └── Token storage
        ├── ~/.config/com.vercel.cli/auth.json (Vercel CLI tokens)
        └── ~/.turbo/config.json (turbo tokens)
```

Supports reading tokens from the Vercel CLI if present, allowing shared authentication.

## Notes

The login flow uses the OAuth 2.0 Device Authorization Grant (RFC 8628) — the user visits a URL and enters a code in the browser. No local server is needed for login. SSO flows still use a one-shot localhost redirect server for the SSO callback. Tokens are stored locally and can be refreshed automatically when expired.
