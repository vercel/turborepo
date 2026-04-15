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
    │   ├── Vercel device-flow team validation
    │   └── Localhost redirect for self-hosted SSO callback
    └── Token storage
        └── ~/.turbo/config.json (Turbo tokens and refresh metadata)
```

Older Vercel CLI auth files may still be read for backward compatibility. When possible, Turbo exchanges that legacy token for a Turbo-scoped token and persists the result in `~/.turbo/config.json`.

## Notes

The login flow uses the OAuth 2.0 Device Authorization Grant (RFC 8628) — the user visits a URL and enters a code in the browser. No local server is needed for Vercel login or Vercel SSO. Self-hosted SSO flows still use a one-shot localhost redirect server for the callback. Tokens are stored locally and can be refreshed automatically when expired.
