# Turborepo Microfrontends Proxy

A local development HTTP proxy for routing requests to multiple microfrontend applications based on path patterns.

## Purpose

This library crate provides the core proxy functionality for Turborepo's microfrontends feature. It enables developers to run multiple applications on different ports and access them all through a single localhost port during development.

## Key Features

- **Path-based routing**: Route requests to different apps based on URL paths
- **Pattern matching**: Support for exact matches, parameters (`:slug`), and wildcards (`:path*`)
- **Error handling**: Beautiful error pages when apps aren't running
- **Zero configuration**: Uses `microfrontends.json` for automatic setup
- **HTTP proxying**: Forward all request headers and stream responses
- **WebSocket support**: Full bidirectional WebSocket proxying for hot module reload (HMR) and real-time features

## How It Works

### 1. Configuration Loading

The proxy reads `microfrontends.json` which defines:

- Applications and their local ports
- Routing patterns for each application
- Proxy server port (default: 3024)

### 2. Request Routing

```
Incoming Request
    ↓
Parse Path
    ↓
Match Against Routing Patterns
    ↓
    ├─ Match Found → Forward to Child App Port
    └─ No Match → Forward to Default App Port
```

### 3. Request Forwarding

**HTTP Requests:**

- Preserve all headers except `Host`
- Add forwarding headers (`X-Forwarded-*`)
- Stream request body to target
- Stream response back to client

**WebSocket Connections:**

- Detect WebSocket upgrade requests (`Upgrade: websocket` header)
- Forward upgrade handshake to target application
- Establish bidirectional proxy for WebSocket frames
- Forward all WebSocket messages (text, binary, ping, pong, close) between client and target
- Automatic cleanup on connection close

### 4. Error Handling

If an application port isn't reachable:

- Return 502 Bad Gateway status
- Show helpful HTML error page with:
  - Which application should be running
  - Port configuration
  - Suggested command to start app
  - Troubleshooting tips

## Path Pattern Matching

### Exact Match

```
Pattern: /blog
Matches: /blog
Does not match: /blog/, /blog/post, /blogs
```

### Parameter Match

```
Pattern: /blog/:slug
Matches: /blog/hello, /blog/world
Does not match: /blog, /blog/hello/comments
```

### Wildcard Match

```
Pattern: /blog/:path*
Matches: /blog, /blog/, /blog/post, /blog/post/123
Does not match: /blogs
```

## Example Configuration

```json
{
  "version": "1",
  "options": {
    "localProxyPort": 3024
  },
  "applications": {
    "web": {
      "development": {
        "local": { "port": 3000 }
      }
    },
    "docs": {
      "development": {
        "local": { "port": 3001 }
      },
      "routing": [
        {
          "paths": ["/docs", "/docs/:path*"]
        }
      ]
    },
    "api": {
      "development": {
        "local": { "port": 3002 }
      },
      "routing": [
        {
          "paths": ["/api/:version/:endpoint"]
        }
      ]
    }
  }
}
```

### Routing Behavior

**HTTP Requests:**

- `http://localhost:3024/` → `http://localhost:3000/`
- `http://localhost:3024/about` → `http://localhost:3000/about`
- `http://localhost:3024/docs` → `http://localhost:3001/docs`
- `http://localhost:3024/docs/api` → `http://localhost:3001/docs/api`
- `http://localhost:3024/api/v1/users` → `http://localhost:3002/api/v1/users`

**WebSocket Connections:**

- `ws://localhost:3024/_next/webpack-hmr` → `ws://localhost:3000/_next/webpack-hmr` (Next.js HMR)
- `ws://localhost:3024/docs/_next/webpack-hmr` → `ws://localhost:3001/docs/_next/webpack-hmr`
- `ws://localhost:3024/api/socket` → `ws://localhost:3002/api/socket`

WebSocket connections follow the same routing rules as HTTP requests based on the path.

## Architecture

```
┌─────────────────────────────────────────────────┐
│  Browser (localhost:3024)                       │
│  HTTP + WebSocket                               │
└────────────────┬────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────┐
│  Proxy Server                                   │
│  ┌───────────────────────────────────────────┐  │
│  │ Router (pattern matching)                 │  │
│  │ • HTTP request forwarding                 │  │
│  │ • WebSocket upgrade detection             │  │
│  │ • Bidirectional frame proxying            │  │
│  └───────────────────────────────────────────┘  │
└────────────────┬────────────────────────────────┘
                 │
        ┌────────┴────────┬────────────┐
        ▼                 ▼            ▼
   ┌─────────┐      ┌─────────┐  ┌─────────┐
   │ App 1   │      │ App 2   │  │ App 3   │
   │ :3000   │      │ :3001   │  │ :3002   │
   │ (+ HMR) │      │ (+ HMR) │  │ (+ WS)  │
   └─────────┘      └─────────┘  └─────────┘
```

## Components

### Router (`router.rs`)

- Parses routing configuration
- Matches request paths against patterns
- Returns target application and port

### ProxyServer (`proxy.rs`)

- Listens on configured port
- Accepts HTTP requests
- Forwards to target applications
- Handles connection errors

### Error Handling (`error.rs`)

- ProxyError types for all failure modes
- ErrorPage builder for HTML error pages
- Helpful troubleshooting information

## Testing

```bash
# Run all tests
cargo test -p turborepo-microfrontends-proxy

# Run with logging
RUST_LOG=debug cargo test -p turborepo-microfrontends-proxy -- --nocapture
```

### Test Coverage

- ✅ Path pattern parsing and matching
- ✅ Router configuration and route selection
- ✅ Error page HTML generation
- ✅ Integration with microfrontends config
- ✅ Multiple child apps with different patterns
- ✅ Edge cases (parameters, wildcards, exact matches)
- ✅ WebSocket upgrade detection
- ✅ WebSocket routing to different applications

## Dependencies

- `hyper` v1.0 - HTTP server and client with upgrade support
- `tokio` - Async runtime
- `tokio-tungstenite` v0.21 - WebSocket protocol implementation
- `futures-util` v0.3 - Utilities for async stream handling
- `turborepo-microfrontends` - Configuration parsing

## Limitations

- **Manual app startup**: Apps must be running before proxy starts
- **No health checks**: Immediate error if app port unreachable

## Future Enhancements

- Auto-start applications if not running
- Health checks and retry logic with backoff
- Request/response logging
- Performance metrics and monitoring
- Connection pooling
- Request timeout configuration
- HTTP/2 support
- Compression for WebSocket messages

## Integration

This library is designed to be integrated into the main `turbo` CLI. It will be invoked when:

1. A `microfrontends.json` file is detected
2. The package does NOT have `@vercel/microfrontends` as a dependency
3. `turbo dev` command is running

The CLI will handle:

- Loading configuration from disk
- Instantiating the proxy server
- Running it alongside development tasks
- Graceful shutdown on Ctrl+C

## License

See the LICENSE file in the repository root.
