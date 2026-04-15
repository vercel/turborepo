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
        └── ~/.turbo/config.json (Turbo tokens and refresh metadata)
```

Older Vercel CLI auth files may still be read to preserve refresh support for existing Turbo sessions, but Turbo no longer writes credentials there.

## Notes

The login flow uses the OAuth 2.0 Device Authorization Grant (RFC 8628) — the user visits a URL and enters a code in the browser. No local server is needed for login. SSO flows still use a one-shot localhost redirect server for the SSO callback. Tokens are stored locally and can be refreshed automatically when expired.
