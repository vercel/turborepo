# Incremental Tasks

## Overview

Incremental tasks allow Turborepo to manage tool-specific incremental cache artifacts (e.g., TypeScript's `.tsbuildinfo`, Rust's `target/debug/incremental/`) across runs and machines via remote cache. This enables faster re-execution on cache misses by restoring prior incremental state before the tool runs.

This system coexists with the existing all-or-nothing output cache. The output cache handles full cache hits (hash match → skip execution). Incremental tasks handle cache misses (hash mismatch → restore incremental state → execute faster).

## User-Facing Contract

By configuring `incremental`, the user asserts that the underlying tool is resilient to receiving bad, partial, stale, or corrupted incremental state. Turbo makes a best-effort attempt to provide useful incremental state but makes no guarantees about its validity. The tool must be able to fall back to a full execution if the incremental state is unusable. If the tool can't cope, that's a misconfiguration — not a Turborepo bug.

## Configuration

The `incremental` field is defined per-task in the root `turbo.json` under `tasks`. It is an array of partition objects. Each partition represents a distinct set of incremental artifacts with its own cache key.

```json
{
  "tasks": {
    "check": {
      "incremental": [
        { "outputs": ["tsconfig.tsbuildinfo"] }
      ]
    },
    "build": {
      "incremental": [
        {
          "inputs": ["rust-toolchain.toml", "Cargo.lock"],
          "outputs": ["target/.fingerprint/**", "target/debug/incremental/**"]
        },
        {
          "inputs": ["babel.config.js"],
          "outputs": [".babel-cache/**"]
        }
      ]
    }
  }
}
```

### Partition Fields

| Field | Required | Description |
|-------|----------|-------------|
| `outputs` | Yes | Glob patterns of incremental artifact files to cache. Supports exclusion patterns (e.g., `["target/**", "!target/tmp"]`). Paths are relative to the package directory, same as regular `outputs`. |
| `inputs` | No | Glob patterns of files that, when changed, invalidate this partition's incremental cache. When omitted, the partition key does not include an input hash. |

### Constraints

- `incremental` is always an array, even for a single partition. No shorthand object form.
- Glob behavior (inclusions, exclusions) is identical to the existing `outputs` and `inputs` globbing.
- Partition output globs may overlap with each other or with the task's regular `outputs`. This is allowed. When partitions overlap, later partitions in the array overwrite files from earlier ones during restore (last-write-wins). Overlap with regular `outputs` means the same files are cached through both mechanisms independently.
- Older Turborepo versions that don't recognize the `incremental` field will fail with a schema validation error. This is expected — incremental tasks require a supporting version.

## Cache Key

Each incremental partition has its own cache key:

- **With `inputs`**: `(package, task, partition_index, hash(partition_inputs))`
- **Without `inputs`**: `(package, task, partition_index)`

These components are serialized into a deterministic composite and hashed with a versioned prefix (`incremental:v1:`). The resulting artifact is stored with the key `incremental-<sha256>` in the remote cache, using the existing artifact storage API (`/v8/artifacts/{key}`). No server-side changes are required.

Branch is intentionally excluded from the cache key. Incremental artifacts are tool-managed caches that the tool validates against current source files. Receiving state from another branch is never wrong, only potentially suboptimal — the tool reconciles the delta. This means all branches within the same team scope share incremental cache entries.

The partition index (0-based position in the `incremental` array) ensures that multiple partitions on the same task get separate cache entries. **Reordering partitions in the `incremental` array invalidates their caches.**

## Lifecycle

### On a Cache Miss

```
1. Compute task hash → full cache miss
2. For each incremental partition:
   a. Check if ANY files matching the output globs exist on disk
      → If yes: skip remote fetch for this partition (local state is fresh enough)
      → If no: continue to remote fetch
   b. Compute partition cache key
   c. Fetch from remote cache
      → Success: extract files to package directory
      → Failure: log warning, proceed without incremental state
3. Execute the task (incremental fetch MUST complete before execution begins)
4. On task success: for each partition, collect output files and upload to remote
5. On task failure: do NOT upload incremental artifacts
```

### Critical Ordering Constraint

Incremental artifact fetch **must complete before task execution begins.** Turbo must never start a task while an incremental fetch is in-flight. This prevents tools from starting without their cache and having it appear mid-execution. If the fetch is slow, turbo waits. If it fails, turbo logs a warning and executes without incremental state.

### On a Full Cache Hit

No incremental behavior. A full cache hit means the task doesn't execute, so incremental state is irrelevant. The on-disk incremental files may become stale, but this is acceptable — the next cache miss will either use the local files (if they exist) or fetch from remote.

## Fallback Chain

When fetching incremental artifacts, turbo uses this priority order:

1. **Local on-disk files** — If any files matching the partition's output globs exist, skip remote entirely.
2. **Remote cache** — Fetch from the partition's cache key.

## Upload Behavior

- Incremental artifacts are uploaded after **every successful task execution**, regardless of whether the artifacts changed from the prior upload.
- Incremental uploads happen in parallel with regular output cache saves. A failure in either does not affect the other.
- Uploads use the same tar.zst format and archiving machinery as regular output cache artifacts.
- Uploads use the same signing and verification heuristics as regular cache artifacts.

## Remote Cache Integration

- Uses the existing `/v8/artifacts/{key}` HTTP API. No server-side changes required.
- Artifact keys are prefixed with `incremental-` to namespace them away from regular output cache entries.
- The remote cache server enforces its own size limits. Turbo does not enforce upload size limits.
- Stale or orphaned remote artifacts (e.g., from deleted branches or removed config) are handled by the server's eviction policies.

## Interaction with Existing Flags and Features

| Flag / Feature | Behavior |
|----------------|----------|
| `--force` | Skips incremental fetch (reads disabled). Uploads still occur on success. Existing on-disk incremental files are not removed. |
| `--no-cache` | Disables incremental entirely, same as regular cache. |
| `--remote-only` | Skips the local file existence check and always fetches from remote. |
| `--dry` | Shows incremental cache status alongside existing cache info, following the same patterns. |
| `--summarize` | Includes incremental restore details per task in the summary output. |
| `turbo watch` | No incremental behavior. `turbo watch` does not create caches today. |
| `turbo prune` | Out of scope. `turbo prune` does not handle incremental configuration in v1. |
| `turbo query` | Incremental configuration is visible alongside other task configuration, not as a separate query surface. |

## Local Cache Eviction

Incremental artifacts stored locally are subject to the same eviction configuration as regular cache artifacts (`cacheMaxAge`, `cacheMaxSize`). No separate eviction settings for incremental artifacts.

## Concurrency

Incremental fetches are sequential within a single task's partitions but happen concurrently across different tasks, following the existing task concurrency model. No special throttling is applied to incremental fetches.

## Error Handling

| Failure | Behavior |
|---------|----------|
| Remote fetch fails (network, timeout, 500) | Warning log. Proceed without incremental state. |
| Remote upload fails | Warning log. Task result is unaffected. |
| Incremental files on disk are corrupt | Tool's responsibility to handle. See User-Facing Contract. |
| Input glob is invalid | Warning log. Partition is skipped entirely (no fetch, no upload). Falling back to a less-specific key would risk cross-config cache collisions. |

## Security

Incremental artifacts use the same security model as regular cache artifacts:

- Same signing and verification via `x-artifact-tag`
- Same authentication and authorization via bearer tokens and team scoping
- Same transport security (HTTPS)

Because incremental cache keys are not content-addressed (unlike regular task cache), any successful execution within the same team scope overwrites the incremental entry. This means artifacts from any branch may be consumed by any other branch. This is by design — the user contract asserts the tool can handle arbitrary prior state.

## Visibility

- Incremental restore status is logged per-task as part of the existing cache status line. On a cache miss where incremental state was restored, the message indicates that incremental state was restored (e.g., alongside the existing "cache miss, executing" message).
- Incremental cache status is included in `--dry` output.
- Incremental partition configuration (outputs/inputs) is included in `--summarize` output. Per-partition restore status is a future enhancement.

## Design Decisions

- **Branch excluded from cache key**: Branch was considered as a key dimension but excluded. Incremental state is tool-validated — receiving state from another branch is suboptimal but never incorrect. This avoids O(branches) cache pollution and simplifies the implementation. Revisit if cross-branch interference causes frequent full rebuilds.
- **Upload minimization**: The current design uploads incremental artifacts on every successful run, even if they haven't changed. Future optimization could diff against the prior upload and skip when unchanged, reducing bandwidth costs — particularly important for large artifacts like Rust incremental state and for high-frequency execution patterns like `turbo watch` if it gains cache support.

## Out of Scope for v1

- `turbo prune` support
- `turbo watch` incremental behavior
- Package-level `turbo.json` overrides for `incremental`
- Upload diffing / change detection
- Custom eviction settings for incremental artifacts
- Server-side incremental artifact awareness (new API endpoints, metadata, cleanup)
