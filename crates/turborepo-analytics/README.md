# turborepo-analytics

## Purpose

Cache usage analytics for Turborepo. Records cache hit/miss events and sends them to the Vercel API in the background. Requires user to be logged in.

## Architecture

```
Cache operation (hit/miss)
    └── AnalyticsSender (channel)
        └── Background worker
            ├── Batches events
            └── Sends to Vercel API
```

Events are buffered and sent in batches to minimize network overhead.

## Notes

Only records cache usage events (hits/misses for filesystem and HTTP cache). Analytics are sent asynchronously to avoid impacting task execution performance.
