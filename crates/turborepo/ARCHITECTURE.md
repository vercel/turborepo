# Turbo Run Architecture

This document serves as a sketch of the architecture of the `turbo run` command

## Overview

A run consists of the following steps:

1. Build a package graph based on the JavaScript package manager settings (and, behind `futureFlags.experimentalCargoWorkspaces`, Cargo workspace crates)
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

### Signal-Driven Shutdown

Graceful shutdown and parent-death cleanup are separate responsibilities.
Graceful shutdown happens while the Turbo process is still alive, so it should
be handled internally by the run and process manager. Parent-death cleanup only
applies when Turbo disappears before Rust cleanup code can run.

- `crates/turborepo-lib/src/commands/run.rs` creates a shared
  `SignalHandler` and does not return until all shutdown subscribers finish
  their cleanup work.
- The handler distinguishes signal-driven shutdown (`ShutdownReason::Signal`)
  from close-driven shutdown (`ShutdownReason::Close`). Normal command
  completion uses the close path to drain subscribers without printing
  signal-specific shutdown UX.
- `crates/turborepo-lib/src/run/mod.rs` registers shutdown subscribers for
  task processes, cache writes, and the microfrontends proxy.
- Task processes are spawned into dedicated process groups so Turbo can signal a
  task and all of its descendants together.
- On the first `SIGINT`/`SIGTERM`, Turbo enters graceful shutdown: it prints a
  shutdown message, forwards `SIGINT` to running tasks, and waits for their
  process groups to exit.
- Turbo must not treat direct-child exit as task-tree exit. Package managers,
  shells, and watch commands can leave descendants running after the leader
  exits, so the process manager should track the process targets it spawned and
  keep Turbo alive until all tracked process groups are gone.
- Close-driven shutdown still flushes cache writes and stops processes, but it
  does not arm signal-specific force-shutdown timers.
- If tasks are still running after 3 seconds, Turbo prints the remaining task
  list. In an interactive terminal it also prompts for a second `Ctrl+C` to
  force shut down. Without a terminal on stdin, Turbo instead prints the
  remaining time before the automatic force shutdown.
- On Unix, a second signal escalates to a force kill. When stdin is not
  attached to a terminal, Turbo auto-escalates after 10 seconds instead.
- On Windows, graceful shutdown falls back to an immediate kill because the
  platform does not support Unix-style signal forwarding to task process
  groups.

Parent-death cleanup is not part of normal graceful shutdown. An in-process map
cannot help after `SIGKILL`, a crash, or OOM because the map dies with Turbo.
Turbo should not start a per-task Unix watchdog for this case. If abnormal
cleanup is required later, prefer a bounded run-level mechanism:

- A run-level reaper, if used, should be owned by `ProcessManager` and shared by
  all tasks in the run.
- Tasks register their process target (`pid`, `pgid`, and session identity) when
  spawned and unregister on normal exit or Turbo-managed shutdown.
- If the Turbo process disappears and the control pipe reaches EOF, the reaper
  can signal the remaining registered process groups and escalate if needed.
- Linux can use `prctl(PR_SET_PDEATHSIG)` as a best-effort no-helper option, but
  it only signals the direct child and cannot provide delayed escalation.
- Windows should continue using job objects for parent-death cleanup.

Regression coverage for shutdown changes should focus on observable lifecycle
behavior:

- A direct child exiting during shutdown must not let Turbo exit while a tracked
  descendant process group is still alive.
- Graceful shutdown must wait for all tracked process groups, not just all
  direct children.
- Forced shutdown must kill stubborn descendants and clear the tracked task
  records.
- End-to-end `turbo run` signal tests should assert that descendants are not
  leaked after force shutdown.
- Existing final-output coverage should continue proving that shutdown keeps the
  UI and log pipeline alive long enough to drain task output.

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
   containing directly affected tasks, their transitive dependents, and all
   transitive dependencies required for execution (upstream tasks needed as
   cache hits)

This differs from the default `--affected` behavior which operates at the
package level (all tasks in changed packages run).

### 2. Package Graph (`crates/turborepo-repository/src/package_graph/`)

Represents the workspace structure and package dependencies:

- Identify package manager being used
- Discovers packages in workspace
- Performs lockfile analysis
- Builds dependency relationships between workspace packages
- Validates that all non-root packages have a `name` field
  (`PackageGraph::validate()`)

The package graph intentionally allows cyclic dependencies between packages —
this aligns with how npm, pnpm, and yarn handle cyclic workspace deps. Cycle
detection is deferred to the task graph layer (engine builder), since
package-level cycles only matter when they produce task-level cycles via
topological (`^`) dependencies.

#### Toolchains (`crates/turborepo-repository/src/toolchain.rs`)

The package graph is generic over language toolchains. A `Toolchain` answers
ecosystem-specific questions — which packages exist, what command a task
runs, what hash wiring a task derives — so graph construction and execution
never branch on a specific ecosystem. All lookups go through the
`ToolchainRegistry` (carried by the `PackageGraph`); `ToolchainId` is an
open string identifier, not a closed enum; and trait methods are
coarse-grained and data-in/data-out, keeping the door open to out-of-process
plugin adapters. JavaScript is the first, production implementation: its
discovery, script command construction, and phantom-task detection flow
through the trait. Machinery that predates the abstraction and has no trait
surface yet (package-manager resolution for dependency splitting, the JS
lockfile closure phase) is documented as known debt in the module.

Toolchain-derived I/O receives the same task-scoped arguments as execution plus
a narrow, platform-aware startup-environment projection keyed by toolchain.
Dependency tasks do not inherit arguments for a different requested task, each
toolchain can observe only the variables it declares, Windows lookup remains
case-insensitive, and every declared pattern automatically participates in task
hashing. Derived outputs distinguish exact/resolved paths from unavailable
automatic resolution.
When outputs are unavailable, the engine disables implicit caching so a log-only
hit cannot suppress execution, while explicit `outputs`, `cache: true`, and
`cache: false` remain authoritative.

#### Experimental Cargo Support (`crates/turborepo-repository/src/cargo.rs`)

Behind `futureFlags.experimentalCargoWorkspaces` in the root turbo.json,
`turbo run` also discovers Rust crates from a Cargo workspace at the repo
root and adds them to the package graph. `CargoToolchain` is the second
`Toolchain` implementation.

Turborepo does not replace Cargo. Cargo is itself a build system with its
own dependency graph, scheduler, and incremental cache (`target/`), so the
division of labor is: **Turborepo decides which crates are in scope and
whether anything changed; Cargo decides how and in what order to build.**

- **Discovery** (`discover_crates`) shells out to `cargo metadata --no-deps`
  — Cargo is the only correct implementation of its own membership semantics
  (member globs, automatic path-dependency members, excludes, target-specific
  dependency tables, renames). Dev-dependency edges that would form a cycle
  are dropped (Cargo permits dev-dep cycles; crate edges must support
  topological `^` ordering). Crate names are validated, and a crate/JS package
  name collision hard-errors. Crate path dependencies are synthesized as
  `workspace:*` specifiers in the toolchain-neutral descriptor, so the existing
  dependency splitter wires crate→crate edges. A second full `cargo metadata
  --locked --all-features` pass validates resolution and every resolved local
  package: automatic in-repository workspace members are supported, while
  excluded/non-member, outside-repository, and root-manifest local packages
  hard-error because Turborepo cannot hash, watch, or prune their sources
  safely.
- **Package shapes**: crates are classified via `CargoPackageKind`.
  *Entrypoints* (crates with `bin`/`cdylib`/`staticlib` targets) are the
  workspace's deliverables. *Libraries* exist in the package graph — so
  `--filter` and `--affected` propagate through them — but their tasks are
  no-ops: Cargo builds them implicitly as part of an entrypoint's closure. A
  synthetic *workspace* package — named by the user via
  `[workspace.metadata] name` in the root Cargo.toml, a hard requirement —
  depends on every crate and hosts workspace-scoped verbs.
- **Execution** (`Toolchain::task_command`, adapted into the provider chain
  by `ToolchainCommandProvider` in `turborepo-task-executor`): entrypoint
  `build`/`run`/`dev` tasks run `cargo <verb> --package=<crate> --locked` — one
  cargo process that builds the crate's whole dependency closure with Cargo's
  own parallelism. Verification verbs run once at workspace scope:
  `<name>#test` → `cargo test --workspace --locked`, `<name>#lint` → `cargo
  clippy --workspace --locked`, etc. `--locked` preserves the dependency
  resolution validated before task hashing. Cargo commands (except `cargo
  run`) share a mutually-exclusive serial group: concurrent cargo processes
  serialize on the build-directory lock anyway, so the executor runs one at a
  time without the "waiting for file lock" noise. Run summaries derive display
  commands from the same verb tables via `Toolchain::task_display_command`, so
  display cannot drift from execution.
- **Hashing** (`Toolchain::derived_task_io`, consumed by
  `turborepo-engine/src/builder/definitions.rs`): entrypoint tasks hash
  their own sources plus their transitive dependency crates' sources
  (flattened, so invalidation doesn't depend on `dependsOn` wiring), the
  workspace files (root `Cargo.toml`, `.cargo/config*`, `rust-toolchain*`),
  and standard Cargo/cc-rs environment inputs: compiler and rustdoc selection
  and flags, Cargo build/profile/target configuration, native compiler and
  archiver settings (including target-qualified forms), and platform SDK
  selection. Arbitrary variables consumed by project-specific build scripts
  remain explicit task `env` configuration. The workspace package hashes all
  crate directories instead of default-hashing the repo root.
  `$TURBO_DEFAULT$` in a Cargo task's `inputs` means "everything turbo
  derives automatically", so extra inputs (e.g. a file embedded via
  `include_str!` from outside any crate directory) are additive.
- **External dependencies** (`turborepo-lockfiles/src/cargo.rs`): locked
  registry/git packages and the compiler itself flow through the same
  external-dependency hash JS packages use
  (`PackageInfo.transitive_dependencies`). Each crate's closure is computed
  from `Cargo.lock` (identity = version + source + checksum, so git rev
  bumps count). Source-qualified lockfile edges distinguish identical
  name/version packages from different registries or git references, so each
  closure follows Cargo's exact resolved package. A dependency bump therefore
  only invalidates crates that actually depend on it. The complete verbose
  compiler identity from `rustc -vV`,
  including its host triple, is resolved from the repo root (so
  `rust-toolchain` overrides apply) and added to every Cargo package's set.
  This prevents compiler releases, operating systems, architectures, or host
  ABIs from sharing native artifact cache entries. Explicit targets selected
  through hashed task arguments, `CARGO_BUILD_TARGET`, or repository Cargo
  configuration remain distinct. Failure to resolve the compiler identity is
  a hard error. Every non-empty Cargo workspace must have a current
  `Cargo.lock`: discovery runs full `cargo metadata --locked --all-features`
  before hashing, then computes per-crate closures. Missing, stale, unparsable,
  or incomplete lockfiles are hard errors. Turborepo never creates or refreshes
  the source lockfile; users do that explicitly with Cargo and commit the
  result.
- **Caching**: task caches store logs plus, for entrypoint builds, the
  deliverables: bins (`target/*/<bin>`) and cdylib/staticlib artifacts
  (`target/*/lib<name>.{so,dylib,a}`, `<name>.{dll,lib}` — all platform
  spellings are emitted; unmatched globs contribute nothing). The profile
  segment is a wildcard, so `--release` and custom profiles cache without
  configuration — pass-through args participate in the task hash, giving
  each profile its own cache entry. Cargo's internal `target/` state is
  deliberately never cached — it is Cargo's own incremental cache, and
  tarballing it fights Cargo instead of leaning on it (it is also
  multi-gigabyte). For fine-grained compile caching, `RUSTC_WRAPPER`
  (sccache) is the sound layer, and it participates in task hashes so
  toggling it invalidates caches. Entrypoint `run` and `dev` tasks default to
  `cache: false`, because a cache hit must not suppress the requested process;
  an explicit turbo.json `cache` setting overrides the toolchain default.

- **Watch mode** (`Toolchain::watch_spec`, consumed by
  `turborepo-lib/src/package_changes_watcher.rs`): each toolchain declares
  its workspace-definition files and build-byproduct directories. For
  Cargo, any `Cargo.toml` or the root `Cargo.lock` triggers full
  rediscovery (the crate set or its edges may have changed), while events
  under the root `target/` directory are dropped — Cargo writes there
  continuously during builds, and the feedback loop must not depend on a
  `.gitignore` entry (`Cargo.toml` files under `target/` are build
  byproducts, not workspace definition). The watcher builds its package
  graph with the same toolchains a run would register, so watch sees the
  same package set. JavaScript declares nothing extra: workspace
  redefinition is caught by the change mapper's conservative
  all-packages fallback. Known gap: the hash watcher's content-hash dedup
  is JS-glob-based, so a no-op save inside a crate re-runs its tasks as a
  fast cache hit rather than being suppressed.

- **Prune** (`Toolchain::prune_plan` / `prune_finalize`, consumed by
  `turborepo-lib/src/commands/prune.rs`): each toolchain reports what a
  self-contained pruned repository needs beyond the copied packages. For
  Cargo: the kept-member set comes from a `Cargo.lock` reachability walk
  (not the package graph — the lockfile merges dev-dependency edges, so
  members reachable only through dev-deps are retained, since kept crates'
  manifests reference them), the lockfile is subset to that closure, and
  the root `Cargo.toml` is rewritten with `toml_edit` (explicit `members`,
  filtered `default-members`, `[workspace.dependencies]` path entries to
  removed crates dropped — comments and formatting preserved). Toolchain
  and Cargo config files are carried over. Reachability pruning cannot see
  Cargo's feature unification, so `prune_finalize` runs `cargo metadata`
  once in the complete output (offline first, then networked) to let Cargo
  minimally sync its own lockfile; failure downgrades to a warning.
  Only toolchains that contributed a prune plan are finalized. Finalizers
  report files they may have changed, and prune copies those finalized bytes
  to alternate output layers without rerunning the toolchain. Reported sources
  must be regular files rather than symlinks, and paths must remain within both
  output roots lexically and after resolving symlinks; invalid paths and
  synchronization failures are warnings. In docker layout,
  the json layer carries the root manifest, each kept crate's `Cargo.toml`, and
  finalized lock; sources go to the full layer. A
  package anchored at the repo root (the synthetic workspace package) is not
  a pruneable target.

- **Compile cache** (`Toolchain::compile_cache_env`, consumed by
  `ToolchainCommandProvider`; gated by `futureFlags.experimentalCargoSccache`):
  when enabled alongside `experimentalCargoWorkspaces` in a CI environment
  with a linked Remote Cache, the run serves a local HTTP proxy
  (`turborepo-sccache-proxy`) that presents an sccache-compatible webdav
  storage backend and translates `GET`/`PUT`/`HEAD` into Remote Cache
  artifact calls. Nothing needs installing: turbo embeds sccache as a
  library (a Vercel fork of mozilla/sccache pinned in Cargo.toml, adding
  an explicit-args entrypoint) and acts as the compiler wrapper itself —
  `main.rs` dispatches invocations marked with `TURBO_SCCACHE_WRAPPER=1`
  (and sccache's internal `SCCACHE_START_SERVER=1` respawn) to
  `sccache::main_from_args`, alongside the LSP and Windows ctrl-c shims.
  Cargo tasks get `RUSTC_WRAPPER=<turbo>`, the wrapper marker,
  `SCCACHE_WEBDAV_ENDPOINT`/`SCCACHE_WEBDAV_TOKEN`, and
  `CARGO_INCREMENTAL=0` injected at execution time; JavaScript injects
  nothing. Objects are fetched lazily per rustc invocation, so nothing is
  restored before a task runs; the two cache layers compose (task-cache
  hit: nothing executes; miss: cargo's conservative recompiles become
  downloads). The endpoint must be stable across runs because the sccache
  background server captures it at startup and outlives the run: the port
  is derived from the repo root and the bearer token is persisted at
  `.turbo/sccache-proxy-token`. Injection is execution-only and does not
  participate in task hashes (a compile cache is output-transparent). The
  toolchain decides how injection composes with the task environment: a
  user-supplied `RUSTC_WRAPPER` or any `SCCACHE_*` variable signals a
  competing compiler-cache configuration and suppresses the whole injected
  set, while an ambient `CARGO_INCREMENTAL` (CI images commonly export
  `=0`) is tolerated — injection proceeds without overriding it. Every
  unmet precondition disables the proxy softly. CI-only by design: cold environments are where a compile cache
  pays off, while local development is served by cargo's own incremental
  compilation — which the injected `CARGO_INCREMENTAL=0` would disable.
  Lifecycle: started in `Run::execute_visitor` before the visitor,
  shut down fire-and-forget after it. The proxy counts the work-unit
  traffic it serves (hits/misses/stores, health-check probe excluded)
  and the run summary footer reports it as a toolchain-agnostic
  "Incremental cache" line — reuse below the task boundary — shown only
  when the run actually exchanged work units.

A `--filter` that names a crate while support is disabled gets an error
hint pointing at the flag. Released turbo versions hard-error on unknown
`futureFlags` keys, so a repo can only adopt the flag once every consumer
(hooks, CI) runs a version that knows it.

End-to-end coverage lives in `crates/turborepo/tests/cargo_workspace_test.rs`
against the `cargo_monorepo` fixture (a mixed npm + Cargo workspace):
graph shape, execution, caching, deliverable restoration, cross-crate
invalidation, lockfile enforcement, unsupported local-package rejection,
uncached `run`/`dev` execution, and the filter hint. `turbo query` serves Cargo
packages through the same graph.

### 3. Task Graph (`crates/turborepo-lib/src/engine/`)

The task graph is a graph of all tasks that will be part of the run and related configuration.

Due to purely historical reasons, this is referenced as "engine" throughout the codebase.

The core task graph consists of:

#### Engine Builder (`crates/turborepo-lib/src/engine/builder.rs`)

- Parses `turbo.json` and other configuration sources to determine task definitions
- Resolves task dependencies (topological `^build` and direct `build`)
- Creates task nodes and dependency edges
- Validates task definitions and is the sole layer that checks for circular
  dependencies (both cycles and self-dependencies in the task graph)
- Resolves each task's `command` override
  (`futureFlags.experimentalTaskCommand`) in one place
  (`resolve_command_override`, `turborepo-engine`'s
  `builder/definitions.rs`), across five precedence levels: Package
  Configuration `command` → root `pkg#task` `command` → package-authored
  script (`Toolchain::authors_task`) → unscoped root default (per-toolchain
  maps fan out by toolchain id) → the toolchain's own resolution. The
  resolved override is authoritative in both directions — an argv executes
  even where the toolchain defines nothing, an opt-out never executes even
  where it does — and feeds global-deps hashing, the TUI task list, the
  executor (`ToolchainCommandProvider`), and the task hash
  (`TaskHashable.commandOverride`/`commandOptOut`). Toolchains place the
  argv in their frame: cwd is the package directory, nothing is prepended,
  and Cargo keeps its serial group when the override still invokes cargo.
  Because an argv override is otherwise arbitrary, it does not inherit the
  native command's toolchain-derived inputs, outputs, default-input behavior,
  or hash environment; its turbo.json `inputs`, `outputs`, and `env` are the
  authoritative task-level I/O configuration. Toolchain task defaults and
  execution-only compile-cache environment injection likewise apply only to
  native toolchain-resolved commands.

#### Engine Execution (`crates/turborepo-lib/src/engine/execute.rs`)

- Orchestrates task execution in topological order
- Enforces user set concurrency limit
- Sends tasks to the visitor for execution
- Handles early termination and error propagation

**Task Graph Structure:**

- Nodes: Individual tasks identified by `TaskId` (package#task) or root
- Root is an artifacts of our Go graph library which required all graphs have a single entrypoint
- Edges: Dependencies between tasks, at the moment no additional data (weights) are added to the edge

#### Engine Pruning (`crates/turborepo-engine/src/lib.rs`)

- `retain_affected_tasks` keeps directly affected tasks, transitive dependents,
  and all transitive dependencies required for normal `--affected` execution
- `create_engine_for_subgraph` and `retain_watch_affected_tasks` are used by
  package-level and task-input watch modes, respectively. They keep changed
  tasks, transitive dependents, and only cacheable upstream dependencies that
  can restore outputs without forcing non-cacheable tasks to rerun. Persistent
  non-interruptible tasks are excluded because watch mode cannot restart them

### 4. Task Visitor (`crates/turborepo-lib/src/task_graph/visitor/`)

The task graph visitor handles task execution:

#### Visitor `visit` (`crates/turborepo-lib/src/task_graph/visitor/mod.rs`)

- Receives tasks from the engine when they can be executed
- Calculates task hashes. Most task hashes are precomputed before scheduling,
  but tasks with structured deferred inputs (`mode: "jit"` or
  `mode: "dependencyOutputs"`) defer final file-input hashing until the engine
  dispatches the task, after its dependencies have completed and restored any
  cached outputs. Tasks that depend on deferred tasks are also deferred so their
  dependency hashes are available before their own hash is calculated. Once a
  deferred task has a real hash, the visitor precomputes any unblocked
  non-deferred descendants instead of waiting for each descendant to be
  dispatched.
- Creates `ExecContext` for each task
- Manages UI output and progress tracking
- Collects errors and execution information

#### Task Executor (`crates/turborepo-lib/src/task_graph/visitor/exec.rs`)

- `ExecContext`: Holds state required to execute a task
- Attempts cache restoration before execution
- Spawns and manages child processes using `turborepo_process`
- Captures `stdout`/`stderr` output
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

Cache restore and storage enforce filesystem boundaries. Restores are anchored
to the selected restore directory, preserve safe symlinks, and reject symlink
targets that escape that anchor. Cache storage rejects task outputs that resolve
outside the repository root.

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

Those prefixes are relative to the repo index root, which is usually the Git
root. This matters when the Turbo root is nested inside a larger Git repository:
the root package should scope to the nested Turbo directory, not request an
untracked walk of the entire parent repository.

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
- **Structured Deferred Inputs**: `inputs` entries with `mode: "jit"` are file
  inputs hashed just before task execution. `mode: "dependencyOutputs"` selects
  already-expanded dependency task nodes and defers the task hash because those
  producers' declared outputs are not known until after dependencies complete.
  In dry runs, these task hashes are reported as deferred.
- **CRLF Normalization**: When `.gitattributes` marks files as `text` or
  `text=auto`, git normalizes CRLF line endings to LF in blob objects. The
  `crlf` module in `turborepo-scm` replicates this so turbo's file hashes
  match git's regardless of the code path (git or manual/no-git after
  `turbo prune`). `.gitattributes` is included in the global hash inputs
  and preserved by `turbo prune`. Known limitations: only root-level
  `.gitattributes` is loaded; `eol=` is not handled.

#### `globalConfiguration` and `global.inputs`

When the `globalConfiguration` future flag is enabled, `global.inputs` (formerly
`globalDependencies`) files are **not** included in the global hash. Instead,
they are prepended as implicit input globs to every task's `TaskInputs` during
engine construction (see `prepend_global_inputs` in
`crates/turborepo-engine/src/task_definition.rs`).

This means:
- The global hash still exists (lockfile, engines, global env, root deps) but
  does not include `global.inputs` file hashes
- Tasks can exclude specific global input files via negation globs
  (e.g. `"inputs": ["$TURBO_DEFAULT$", "!$TURBO_ROOT$/tsconfig.json"]`)
- Tasks with no explicit `inputs` key get `default: true` set so package files
  are still hashed alongside the global inputs

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
package/task graph).

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
`query_server`; the `turbo query` command handler uses it for direct query
execution and the local GraphQL server mode.

## Data Flow Overview

### 1. Task Graph Building

```
RunBuilder
  ├── Package Discovery → PackageGraph (validates package names)
  ├── Task Discovery → EngineBuilder
  ├── Task Graph Construction → Engine (built)
  └── Task Graph Validation (cycles, missing deps) → Ready Engine
```

**Process:**

1. Discover packages and build package dependency graph
2. Load turbo.json configurations for tasks
3. Create task nodes for each package × task combination
4. Build dependency edges based on `dependsOn` configurations
5. Validate task graph for cycles and missing dependencies

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

#### Incremental Cache (`crates/turborepo-run-cache/src/incremental.rs`)

Handles tool-managed incremental artifacts (e.g., `.tsbuildinfo`) that persist
across runs via remote cache, speeding up cache misses by restoring prior
incremental state before execution.

- Gated behind the `incrementalTasks` future flag
- Operates per-partition with independent cache keys
- Fetch completes before task execution begins (strict ordering)
- Upload happens after successful execution, in parallel with regular cache save
- All blocking filesystem operations run on `spawn_blocking` threads
- See `SPEC.md` for full specification

```
On Cache Miss:
  Visitor.visit()
    ├── Calculate Hash → Cache Miss
    ├── Fetch Incremental Artifacts (sequential per-partition, must complete before exec)
    ├── Execute Task
    ├── Save to Cache
    ├── Upload Incremental Artifacts (concurrent per-partition, parallel with cache save)
    └── Track Results
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

OTEL endpoints must be HTTPS URLs without userinfo. Literal private, loopback, link-local, multicast, documentation, carrier-grade NAT, and known metadata-service IP endpoints are rejected; use `localhost` by name for local collectors.

#### Metrics Emitted

- `turbo.run.duration_ms` - Run duration histogram
- `turbo.run.tasks.attempted` - Tasks attempted counter
- `turbo.run.tasks.failed` - Tasks failed counter
- `turbo.run.tasks.cached` - Cache hit counter
- `turbo.task.duration_ms` - Per-task duration histogram (when `taskDetails` enabled)
- `turbo.task.cache.events` - Per-task cache events (when `taskDetails` enabled)

Duration histograms use custom millisecond buckets sized for build and task durations, rather than the OpenTelemetry SDK's default latency buckets.

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

### 9. User-Facing Logging (`crates/turborepo-log/`)

Structured event system for messages intended for end users (warnings,
errors, informational output). Distinct from `tracing`, which remains
for developer diagnostics.

#### Key Types

- `Logger` — Dispatches events to registered sinks. Set globally via
  `init()` (once, at startup) or used directly via `Logger::handle()`
  for testing.
- `LogHandle` — Source-scoped handle for emitting events. Created via
  `log()` (global) or `Logger::handle()` (specific logger). Resolves
  the global logger at `.emit()` time, not at handle or builder
  creation time — handles and builders created before `init()` work
  once the global logger is set.
- `LogSink` — Trait for event destinations. Built-in sinks:
  `CollectorSink` (in-memory buffer for post-run summaries) and
  `FileSink` (newline-delimited JSON with optional size limiting).
- `LogEvent` — Structured event with level, source, message, typed
  fields, and timestamp.

#### Relationship to `turborepo-ui`

`turborepo-ui` handles terminal rendering (TUI, console formatting).
`turborepo-log` handles structured event capture and dispatch. A
terminal sink in `turborepo-ui` can implement `LogSink` to bridge
events into the rendering pipeline. `turborepo-log` intentionally has
no dependency on `turborepo-ui` — it sits at the bottom of the
dependency graph.

#### Data Flow

```
Subsystem / Task Executor
  └── LogHandle.warn("msg").field("k", v).emit()
        └── Logger.emit(&event)
              ├── CollectorSink → in-memory buffer → post-run summary
              └── FileSink → JSONL file → external tooling
```
