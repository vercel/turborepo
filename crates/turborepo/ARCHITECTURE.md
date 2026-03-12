# Turbo Run Architecture

This document serves as a sketch of the architecture of the `turbo run` command

## Overview

A run consists of the following steps:

1. Build a package graph based on the Javascript package manager settings
2. Build a task graph based on package dependencies and configuration
3. Determine global/task hashes
4. Execute tasks in topological order
   1. Attempt to restore outputs from cache
   2. Execute task
   3. Cache task outputs for future runs in background
5. Collect and summarize execution results

## Entry Point

- **CLI Entry**: `crates/turborepo/src/main.rs` - Constructs `TurboQueryServer` (the concrete `QueryServer` implementation) and passes it to `turborepo_lib::main`
- **Command Handler**: `crates/turborepo-lib/src/commands/run.rs` - Entry point for the run command, sets up signal handling and UI
- **Main Logic**: `crates/turborepo-lib/src/run/mod.rs` - Core run implementation

## Core Architecture Components

### 1. Run Builder (`crates/turborepo-lib/src/run/builder.rs`)

**Key responsibilities:**

- Package discovery and lockfile analysis
- Task filtering based on arguments (task names and `--filter`)
- Root task scoping via `FilterMode` (from `turborepo-types`): when no filter
  or only exclude filters are active, root tasks defined in `turbo.json` are
  auto-included. Explicit include filters or `--affected` suppress root task
  injection. See `calculate_filtered_packages` and `FilterMode`.
- Task graph construction and validation
- Task-level affected detection (see below)
- Cache setup (local and remote)
- Activating shared HTTP client initialization once telemetry, remote cache, or
  linked analytics are known to be needed
- Building a tracked repo index eagerly, then augmenting it with scoped
  untracked-file discovery once the selected package set is known
- Producing a final `Run` struct ready for execution

#### Task-Level Affected Detection

When the `affectedUsingTaskInputs` future flag is enabled and `--affected` is
active, the run builder applies a second filtering pass after engine
construction:

1. **File change detection**: SCM provides the set of changed files between refs
2. **Task input matching** (`turborepo-types/src/task_input_matching.rs`): Each
   task's `inputs` globs are compiled and checked against the changed files.
   Shared with `turbo query { affectedTasks }`.
3. **Task change detection** (`turborepo-lib/src/task_change_detector.rs`):
   Determines directly affected tasks, handling global deps and per-task inputs
4. **Engine pruning** (`Engine::retain_affected_tasks`): Returns a new engine
   containing only directly affected tasks plus their transitive dependents

This differs from the default `--affected` behavior which operates at the
package level (all tasks in changed packages run).

### 2. Package Graph (`crates/turborepo-repository/src/package_graph/`)

Represents the workspace structure and package dependencies:

- Identify package manager being used
- Discovers packages in workspace
- Performs lockfile analysis
- Builds dependency relationships between workspace packages

### 3. Task Graph (`crates/turborepo-lib/src/engine/`)

The task graph is a graph of all tasks that will be part of the run and related configuration.

Due to purely historical reasons, this is referenced as "engine" throughout the codebase.

The core task graph consists of:

#### Engine Builder (`crates/turborepo-lib/src/engine/builder.rs`)

- Parses `turbo.json` and other configuration sources to determine task definitions
- Resolves task dependencies (topological `^build` and direct `build`)
- Creates task nodes and dependency edges
- Validates task definitions and checks for circular dependencies

#### Engine Execution (`crates/turborepo-lib/src/engine/execute.rs`)

- Orchestrates task execution in topological order
- Enforces user set concurrency limit
- Sends tasks to the visitor for execution
- Handles early termination and error propagation

**Task Graph Structure:**

- Nodes: Individual tasks identified by `TaskId` (package#task) or root
- Root is an artifacts of our Go graph library which required all graphs have a single entrypoint
- Edges: Dependencies between tasks, at the moment no additional data (weights) are added to the edge

### 4. Task Visitor (`crates/turborepo-lib/src/task_graph/visitor/`)

The task graph visitor handles task execution:

#### Visitor `visit` (`crates/turborepo-lib/src/task_graph/visitor/mod.rs`)

- Receives tasks from the engine when they can be executed
- Calculates task hashes
- Creates `ExecContext` for each task
- Manages UI output and progress tracking
- Collects errors and execution information

#### Task Executor (`crates/turborepo-lib/src/task_graph/visitor/exec.rs`)

- `ExecContext`: Holds state required to execute a task
- Attempts cache restoration before execution
- Spawns and manages child processes using `turborepo_process`
- Captures `stdout`/`sterr` output
- Saves outputs to cache on success
- Reports task result back to the execution engine

**Execution Flow:**

1. Check cache for existing results
2. If cache miss, execute the task
3. Capture outputs and logs
4. Save results to cache (if successful)
5. Report status back to engine

### 5. Caching System (`crates/turborepo-lib/src/run/cache.rs` and `crates/turborepo-cache/`)

Multi-layered caching system:

#### Cache Hierarchy

1. **Local FS Cache**: Fast local file system cache
2. **Remote Cache**: Shared cache (typically Vercel's service)
3. **Cache Multiplexer**: Wraps local and remote to provide single cache to check

#### Task Cache Flow

1. **Cache Lookup**: Check local cache first, then remote
2. **Cache Restoration**: Extract and restore cached files
3. **Cache Storage**: Compress and store task outputs
4. **Cache Metadata**: Track cache hits, timing, and sources

#### Key Components

- `RunCache`: High-level cache coordination
- `TaskCache`: Individual task cache management
- `AsyncCache`: Handles async cache operations. Supports both local filesystem and remote HTTP caches
- `SharedHttpClient`: Process-wide lazy/activatable `reqwest::Client`
  initialization shared by telemetry and remote-cache consumers

#### Shared HTTP Client Initialization

Network consumers do not construct an HTTP client speculatively at process
startup. Instead:

1. The CLI and run builder determine whether telemetry, remote cache, or linked
   analytics will actually need networking for the current invocation
2. Once that need is known, they activate shared client initialization
   immediately so TLS setup overlaps with other startup work
3. Telemetry flushes and remote-cache operations both reuse the same initialized
   `reqwest::Client`

This avoids paying client/TLS setup on invocations with no network use while
still warming the client before the first network request in the common case.

#### Two-Stage Repo Index Construction

`turbo run` builds SCM state in two stages:

1. A background startup task reads `.git/index` and records committed blob IDs
   plus modified/deleted tracked files for the whole repo
2. After package filtering finishes, Turborepo computes the package roots it
   actually needs for hashing and augments that tracked index with untracked
   files only for those prefixes

This keeps the cheap tracked-index work overlapped with other startup work while
avoiding a repo-wide untracked walk when only a subset of packages will be
hashed.

#### Worktree Cache Sharing

When running in a Git linked worktree (created via `git worktree add`), Turborepo automatically shares the local file system cache with the main worktree. This enables:

- **Cache hits across worktrees**: Builds on different branches share cache artifacts
- **Reduced disk usage**: Avoids duplicate cache entries across worktrees
- **Faster iteration**: Switching between feature branches benefits from existing cache

**How it works:**

1. `WorktreeInfo::detect()` in `turborepo-scm` determines if the current directory is a linked worktree using Git commands (`git rev-parse --show-toplevel` and `git rev-parse --git-common-dir`)
2. If in a linked worktree, `ConfigurationOptions::resolve_cache_dir()` returns the main worktree's `.turbo/cache` directory instead of the local one
3. Users are notified via the run prelude message: "Remote caching {status}, using shared worktree cache"

**Configuration:**

- Setting an explicit `cacheDir` in `turbo.json` disables worktree cache sharing
- Detection failures (non-git repos, git errors) gracefully fall back to local cache

#### Atomic Cache Writes

Cache writes use an atomic write pattern (write-to-temp-then-rename) for concurrent safety:

1. Cache archives are written to temporary files (`.{filename}.{pid}.{counter}.tmp`)
2. On successful completion, temp files are atomically renamed to final destination
3. `CacheWriter` implements `Drop` to clean up temp files if `finish()` is not called (e.g., on error or panic)

This ensures concurrent readers never see partially written cache files.

### 6. Task Hashing (`crates/turborepo-lib/src/task_hash/`)

Creates a "content identifier" for a specific task depending on current state of inputs:

#### Hash Inputs

- **Global Hash**: Package manager lockfile, global dependencies, environment variables
- **Task Hash**: Task definition, package dependencies, input files, environment variables
- **File Hashing**: Uses git for tracking file changes efficiently
- **Explicit Inputs**: When tasks use custom `inputs`, glob matches still walk the
  filesystem, but clean tracked matches reuse blob OIDs from the repo index
  instead of re-hashing file contents

#### Hash Calculation

- Combines global and task-specific inputs
- Calculated by leveraging `capnp` to serialize in memory structs for hashing
- Artifact of ensuring shared hashing logic between Go and Rust

### 7. Run Tracking and Summary (`crates/turborepo-lib/src/run/summary/`)

The summary module is responsible for any time of summary:

- The "FULL TURBO" summary block at the end of a run
- The summary produced by `--summarize`
- Dry run output `--dry=json`

#### Run Tracker (`crates/turborepo-lib/src/run/summary/mod.rs`)

- Tracks overall run metadata (start time, command, etc.)
- Coordinates task tracking across execution
- Takes final result from `Visitor::visit`
- Generates final run summary

#### Task Tracker (`crates/turborepo-lib/src/run/summary/execution.rs`)

- Tracks individual task execution states
- Records timing, exit codes, and cache status
- Receives information about tasks in real time

#### Summary Generation

- Stitches together result from visitor and the task tracker
- Constructs final summary depending on user ask e.g. `--dry=json`/`--summarize`

### 8. Query Subsystem

The query subsystem powers `turbo query` (GraphQL introspection of the
package/task graph) and the Web UI mode (`--ui=web`).

**Crate layout:**

- `turborepo-query-api` — Trait definitions (`QueryServer`, `QueryRun`) and
  shared error/result types.  `turborepo-lib` depends on this thin interface
  crate instead of the heavy implementation.
- `turborepo-query` — GraphQL implementation using async-graphql, axum, and
  oxc.  Implements the resolvers and HTTP server.
- `turborepo/src/main.rs` — Wires the two halves together via `TurboQueryServer`,
  which implements `QueryServer` by delegating to `turborepo-query`.

**Data flow:** `main()` constructs `Arc<TurboQueryServer>` → passes to
`turborepo_lib::main` → threaded through `shim` → `cli::run` →
`commands::run` → `RunBuilder` → `Run`.  The `Run` struct stores the
`query_server` and uses it in `start_web_ui()` and the `turbo query`
command handler.

## Data Flow Overview

### 1. Task Graph Building

```
RunBuilder
  ├── Package Discovery → PackageGraph
  ├── Task Discovery → EngineBuilder
  ├── Task Graph Construction → Engine (built)
  └── Validation → Ready Engine
```

**Process:**

1. Discover packages and build package dependency graph
2. Load turbo.json configurations for tasks
3. Create task nodes for each package × task combination
4. Build dependency edges based on `dependsOn` configurations
5. Validate graph for cycles and missing dependencies

### 2. Task Graph Traversal

```
Engine.execute()
  ├── Walker (topological order)
  ├── Semaphore (concurrency control)
  ├── Engine -[Task to Run]→ Visitor
  └── Engine ←[Task Result]- Visitor
```

**Process:**

1. `Walker` traverses graph in topological order
2. Semaphore controls maximum concurrent tasks
3. Each ready task is sent to the `Visitor`
4. `Visitor` executes task and reports back to `Engine`
5. Walker continues with newly available tasks

### 3. Task Execution

```
Visitor.visit()
  ├── Calculate Hash
  ├── Check Cache → Cache Hit? → Restore & Done
  ├── Execute Task → Create ExecContext and `exec_context.exec()`
  ├── Save to Cache
  └── Track Results
```

**Process:**

1. Calculate task hash from inputs
2. Check local then remote cache
3. If cache hit: restore outputs and logs
4. If cache miss: execute task command
5. Capture outputs and logs during execution
6. Save results to cache (if successful)
7. Track timing and results

### 4. Cache Operations

```
TaskCache.restore_outputs()
  ├── Check caching disabled?
  ├── Local Cache → exists?
  ├── Remote Cache → exists?
  ├── Fetch & Extract
  └── Return metadata

TaskCache.save_outputs()
  ├── Collect output files
  ├── Compress to tar
  ├── Save to Local Cache
  └── Upload to Remote Cache (async)
```

### 5. Data Collection and Summary

```
RunTracker
  ├── Task Events → ExecutionTracker
  ├── State Aggregation → SummaryState
  ├── Summary Generation → RunSummary
  └── Output (JSON/Console)
```

**Process:**

1. Each task sends lifecycle events (start, success, failure, cache hit)
2. `ExecutionTracker` aggregates state across all tasks
3. Final summary includes timing, cache status, errors
4. Summary is saved to `.turbo/runs/` and optionally printed

### 8. Observability (`crates/turborepo-run-summary/src/observability/` and `crates/turborepo-otel/`)

The observability subsystem enables exporting run metrics to external backends via OpenTelemetry.

#### Architecture

The system uses a two-layer design:

1. **`turborepo-otel`**: Low-level OTLP exporter crate
   - Manages the OpenTelemetry SDK meter provider and instruments
   - Supports gRPC and HTTP/Protobuf protocols
   - Handles connection lifecycle and metric flushing

2. **`turborepo-run-summary/observability`**: Integration layer
   - Provides a `RunObserver` trait for pluggable backends
   - Converts `RunSummary` data into metrics payloads
   - Enabled via the `otel` feature flag

#### Main Components

- `observability::Handle`: Main entry point; wraps backend-specific implementations
- `RunObserver` trait: Abstraction allowing future backends (Prometheus, etc.)
- `OtelObserver`: OpenTelemetry implementation of `RunObserver`

#### Configuration

Observability is configured via `experimentalObservability.otel` in `turbo.json`:

```jsonc
{
  "futureFlags": {
    "experimentalObservability": true
  },
  "experimentalObservability": {
    "otel": {
      "enabled": true,
      "protocol": "http/protobuf",
      "endpoint": "https://otel-collector.example.com:4318/v1/metrics",
      "resource": {
        "service.name": "turborepo"
      },
      "metrics": {
        "runSummary": true,
        "taskDetails": true,
        "runAttributes": {
          "id": false,        // turbo.run.id — unbounded cardinality
          "scmRevision": false // turbo.scm.revision — unbounded cardinality
        },
        "taskAttributes": {
          "id": false,    // turbo.task.id
          "hashes": false // turbo.task.hash, turbo.task.external_inputs_hash — unbounded
        }
      }
    }
  }
}
```

Configuration can also be set via environment variables (`TURBO_EXPERIMENTAL_OTEL_*`) or CLI flags (`--experimental-otel-*`).

#### Metrics Emitted

- `turbo.run.duration_ms` - Run duration histogram
- `turbo.run.tasks.attempted` - Tasks attempted counter
- `turbo.run.tasks.failed` - Tasks failed counter
- `turbo.run.tasks.cached` - Cache hit counter
- `turbo.task.duration_ms` - Per-task duration histogram (when `taskDetails` enabled)
- `turbo.task.cache.events` - Per-task cache events (when `taskDetails` enabled)

Attributes with unbounded cardinality (unique run IDs, Git SHAs, content hashes) are gated behind `runAttributes` and `taskAttributes` config flags, all defaulting to `false`. See the `Metric Attributes and Cardinality` section in `crates/turborepo-otel/src/lib.rs` for the full attribute inventory.

#### Data Flow

```
RunSummary.finish()
  ├── observability::Handle.record(&summary)
  │     ├── Convert to RunMetricsPayload
  │     └── Record via OpenTelemetry instruments
  └── observability::Handle.shutdown()
        └── Flush pending metrics to backend
```
