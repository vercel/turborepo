# turborepo-microfrontends-proxy

## Purpose

Local development proxy for microfrontends. Routes requests to the appropriate dev server based on URL patterns.

## Architecture

```
Incoming request
    └── turborepo-microfrontends-proxy
        ├── Router (URL pattern matching)
        ├── HTTP proxy
        └── WebSocket proxy
            └── Target dev server
```

Key components:
- `Router` - Maps URL patterns to backend servers
- `ProxyServer` - HTTP/WebSocket proxy implementation
- Error pages for routing failures

## Notes

Enables running multiple microfrontend dev servers simultaneously with a single entry point. Handles both HTTP requests and WebSocket connections for HMR.
