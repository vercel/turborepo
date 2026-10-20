# turborepo-engine

## Purpose

Task graph construction and execution orchestration. Builds the dependency graph of tasks and coordinates their parallel execution.

## Architecture

```
turbo.json + package graph
    └── EngineBuilder
        └── Task Graph (petgraph)
            ├── Nodes: tasks (package#task)
            ├── Edges: dependencies
            └── Parallel execution via topological traversal
```

Key components:
- `EngineBuilder` - Constructs task graph from configuration
- `TurboJsonLoader` - Loads and merges turbo.json files
- Execution visitor pattern for graph traversal
- Validation for cycles, missing tasks, invalid dependencies

Outputs:
- DOT format for Graphviz
- Mermaid diagrams
- Direct execution

## Notes

The engine is the core orchestrator for `turbo run`. It resolves task dependencies across packages, validates the graph, and drives parallel execution while respecting dependency ordering.
