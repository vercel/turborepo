# turborepo-graph-utils

## Purpose

Graph utilities built on `petgraph`. Provides transitive closure calculation and cycle detection with suggestions for breaking cycles.

## Architecture

```
petgraph Graph
    └── turborepo-graph-utils
        ├── transitive_closure() - All reachable nodes
        ├── Cycle detection
        └── Cut candidates for breaking cycles
```

## Notes

Used throughout Turborepo for dependency graph analysis. The cycle detection includes helpful error messages showing the cycle path and potential edges to remove.
