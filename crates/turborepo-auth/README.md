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
        ├── ~/.turbo/auth.json (Turbo-managed OAuth sessions)
        └── ~/.turbo/config.json (Legacy/manual tokens and config)
```

Older Vercel CLI auth files may still be read for backward compatibility. When possible, Turbo exchanges that legacy token for a Turbo-scoped token and persists the result in `~/.turbo/auth.json` so older Turbo releases do not try to reuse OAuth access tokens from the legacy config slot.

## Notes

The login flow uses the OAuth 2.0 Device Authorization Grant (RFC 8628) — the user visits a URL and enters a code in the browser. No local server is needed for Vercel login or Vercel SSO. Self-hosted SSO flows still use a one-shot localhost redirect server for the callback. Tokens are stored locally and can be refreshed automatically when expired.
