# WebSocket Implementation for Microfrontends Proxy

## Overview

This document describes the WebSocket support implementation added to the Turborepo microfrontends proxy. WebSocket support enables real-time features like hot module reload (HMR) to work seamlessly across multiple microfrontend applications.

## Implementation Details

### 1. Dependencies Added

- **`tokio-tungstenite` v0.21**: WebSocket protocol implementation
- **`futures-util` v0.3**: Utilities for async stream handling (SinkExt, StreamExt)

### 2. Core Components

#### WebSocket Detection (`is_websocket_upgrade`)

Detects WebSocket upgrade requests by checking for:

- `Upgrade: websocket` header
- `Connection: Upgrade` header (case-insensitive, comma-separated values supported)

#### Connection Handling

- Modified HTTP/1.1 connection builder to support upgrades using `.with_upgrades()`
- Changed `handle_request` to accept `mut req` to allow capturing upgrade futures
- Separate code paths for HTTP and WebSocket requests

#### WebSocket Forwarding (`forward_websocket`)

1. Forwards the WebSocket upgrade request to the target application
2. Captures upgrade futures for both client and server connections
3. Returns the upgrade response to complete the handshake
4. Spawns a background task to handle bidirectional frame forwarding

#### Bidirectional Proxy (`proxy_websocket_connection`)

- Upgrades both client and server connections to WebSocket
- Creates separate send/receive streams for both connections
- Forwards frames in both directions:
  - Client → Server: All frames including close frames
  - Server → Client: All frames including close frames
- Handles connection cleanup automatically
- Logs connection lifecycle events

### 3. Routing

WebSocket connections use the same path-based routing as HTTP requests:

- `/` → Default application
- `/docs/*` → Documentation application
- `/api/*` → API application

This means HMR and other WebSocket connections are automatically routed to the correct application based on the request path.

## Usage Example

### Next.js HMR

With the following configuration:

```json
{
  "applications": {
    "web": {
      "development": { "local": { "port": 3000 } }
    },
    "docs": {
      "development": { "local": { "port": 3001 } },
      "routing": [{ "paths": ["/docs", "/docs/:path*"] }]
    }
  }
}
```

WebSocket connections work automatically:

- `ws://localhost:3024/_next/webpack-hmr` → `ws://localhost:3000/_next/webpack-hmr`
- `ws://localhost:3024/docs/_next/webpack-hmr` → `ws://localhost:3001/docs/_next/webpack-hmr`

## Testing

### Unit Tests

- `test_websocket_detection`: Verifies WebSocket header detection
- `test_websocket_routing`: Confirms WebSocket connections are routed correctly

### Integration Tests

All existing HTTP tests continue to pass, confirming backward compatibility.

## Benefits

1. **HMR Support**: Hot module reload works across all microfrontend applications
2. **Real-time Features**: WebSocket-based features (live updates, notifications) work seamlessly
3. **Transparent Routing**: WebSocket connections follow the same routing rules as HTTP
4. **Error Handling**: Connection errors are logged and handled gracefully
5. **No Configuration Changes**: Existing configurations work without modification

## Technical Notes

### Upgrade Handling

The implementation uses Hyper's upgrade mechanism:

- `hyper::upgrade::on(&mut req)` captures the client upgrade future
- `hyper::upgrade::on(&mut response)` captures the server upgrade future
- Both futures resolve when the connections are upgraded
- Upgraded connections are then converted to WebSocket streams

### Frame Forwarding

Uses `tokio-tungstenite` for WebSocket protocol handling:

- Frames are forwarded as-is (no inspection or modification)
- Both text and binary frames are supported
- Ping/pong frames are forwarded automatically
- Close frames trigger connection cleanup

### Performance Considerations

- Zero-copy frame forwarding where possible
- Separate async tasks for each direction to maximize throughput
- Automatic cleanup prevents resource leaks
- Logging can be disabled in production for performance

## Future Enhancements

Potential improvements for future iterations:

1. **Message Compression**: Optional WebSocket compression support
2. **Connection Pooling**: Reuse WebSocket connections to target applications
3. **Metrics**: Track WebSocket connection count, message rates, etc.
4. **Timeout Configuration**: Configurable idle timeouts for WebSocket connections
5. **Protocol Extensions**: Support for WebSocket extensions (permessage-deflate, etc.)

## Backward Compatibility

This implementation is fully backward compatible:

- HTTP requests work exactly as before
- No configuration changes required
- Existing tests pass without modification
- No breaking API changes
