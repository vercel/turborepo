# turborepo-signals

## Purpose

Signal handling infrastructure. Provides a mechanism to subscribe to signals and coordinate graceful shutdown across multiple components.

## Architecture

```
Signal source (e.g., SIGINT)
    └── SignalHandler
        ├── Subscribers notified
        └── SubscriberGuard ensures completion
```

Key types:
- `SignalHandler` - Central coordinator for signal events
- `SignalSubscriber` - Receives notification when signal occurs
- `SubscriberGuard` - Held until subscriber finishes cleanup

## Notes

Subscribers are notified when a signal occurs or when `close()` is called. The guard pattern ensures all subscribers complete their cleanup before shutdown proceeds.
