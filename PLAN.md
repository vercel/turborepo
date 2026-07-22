# Experimental Rust Support Refactor Plan

## Goal

Improve the internal design of experimental Cargo workspace support without
changing its user experience. Preserve task IDs, commands, filtering behavior,
cache hashes, output paths, dry-run JSON, and diagnostics throughout the work.

## Findings

### 1. Discovery state is unsafe across rediscovery

Status: Resolved.

`CargoToolchain` previously maintained two mutex-protected stores in
`crates/turborepo-repository/src/cargo.rs`. Removed crates could remain after
rediscovery, and readers could observe mismatched package and workspace state.

The toolchain now builds an immutable `CargoModel`, serializes concurrent
discovery calls, and publishes the complete model atomically after successful
discovery. Successful memberless discovery clears prior state, while failed
rediscovery preserves the last successful snapshot.

### 2. Toolchain setup is duplicated and inconsistent

Run, watch, daemon, prune, and engine setup each construct Cargo support
independently. Cargo-only run works, but watch and prune still require a root
`package.json`.

Relevant locations:

- `crates/turborepo-lib/src/run/builder.rs`
- `crates/turborepo-lib/src/run/watch.rs`
- `crates/turborepo-lib/src/commands/daemon.rs`
- `crates/turborepo-lib/src/commands/prune.rs`
- `crates/turborepo-lib/src/package_changes_watcher.rs`

### 3. Cargo task policy is stringly and distributed

Task registration, command construction, argument forwarding, caching,
serialization, and entrypoint selection are implemented through separate verb
tables and conditionals. Adding or changing a task requires coordinated edits
that compilation cannot verify.

### 4. The Cargo module owns too many subsystems

`crates/turborepo-repository/src/cargo.rs` contains discovery, compiler probing,
task policy, hashing, output inference, configuration trust, watching, pruning,
and their tests. This is responsibility mixing rather than merely a large file.

### 5. Package types permit invalid states

`CargoPackageDetails` represents both real crates and the synthetic workspace
using a package kind plus sentinel values. In particular, an empty directory
means the workspace package, while a failed crate-path conversion also silently
produces an empty directory.

### 6. Cache-safety decisions are opaque

Output inference combines numerous booleans and returns `Option`, losing the
reason an output layout could not be trusted. Input trust and output
predictability are distinct concerns but are represented through overlapping
flags and branches.

### 7. Generic package abstractions remain JavaScript-shaped

Cargo packages synthesize `PackageJson` values and `workspace:*` dependencies
to pass through JavaScript dependency splitting. This works, but leaks npm
modeling into Cargo discovery and creates package-graph special cases.

### 8. Cargo task-selection policy leaks into run orchestration

`RunBuilder` derives a `prefer_workspace` boolean from filter modes and applies
toolchain entrypoint selection in two phases. The boolean does not fully express
aggregate, filtered, affected, task-level, or package-qualified selection.

### 9. Compile-cache abstractions are sccache-specific

`CompileCacheEndpoint` is nominally generic but exposes wrapper and server-port
concepts specific to sccache. `Run` also owns the concrete sccache lifecycle.
There is no complete end-to-end test of feature gating, proxy startup, task
execution, statistics, and shutdown.

### 10. Documentation has drifted

Generated schema text still describes library tasks as no-ops, while current
behavior supports filtered library build and verification tasks.

## Implementation Plan

### Phase 1: Stabilize lifecycle and repository composition

Add characterization tests for:

- Repeated discovery after removing or renaming a crate
- Rediscovery of a memberless workspace
- Failed rediscovery preserving the last successful snapshot
- Cargo-only watch initialization and rediscovery
- Cargo-only prune

Replace the independently mutable package and workspace stores with one
immutable snapshot published atomically after all discovery work succeeds. A
possible shape is:

```rust
struct CargoModel {
    workspace: CargoWorkspace,
    packages: HashMap<String, CargoPackage>,
}
```

The immutable discovery snapshot and its regression coverage are complete. The
remaining work in this phase is repository composition and Cargo-only
watch/prune coverage.

Centralize these repository-level decisions:

- Whether Cargo support is enabled and applicable
- Whether a missing root `package.json` is valid
- Which toolchains are registered
- Which package-graph builder constructor is used

Use the same composition path in run, watch, daemon, prune, and query or engine
setup. Preserve existing malformed-`package.json` errors and feature-flag
diagnostics.

### Phase 2: Strengthen the Cargo domain model

Replace `CargoPackageKind` plus sentinel fields with types that encode valid
states:

```rust
enum CargoPackage {
    Crate(CargoCratePackage),
    Workspace(CargoWorkspacePackage),
}
```

Real crates must have typed repository-relative directories. Workspace packages
must not be able to carry crate deliverables or compilation dependencies.
Libraries must not accidentally become runnable entrypoints.

Remove the path-conversion `unwrap_or_default()`. Discovery should either prove
the path is valid or return an actionable error.

Treat a Cargo lockfile member missing from the discovered workspace model as an
explicit prune error rather than emitting a knowingly inconsistent output.

### Phase 3: Consolidate Cargo task semantics

Introduce one typed task catalog that describes:

- Turborepo task name
- Cargo subcommand
- Applicable package roles
- Argument-forwarding behavior
- Default cache behavior
- Serial execution group
- Unfiltered selection policy

Aliases such as `lint` and `clippy`, `doc` and `docs`, and `run` and `dev` should
remain distinct Turborepo task names while sharing Cargo behavior.

Add table-driven tests that cover, for every task and package role:

- Registration
- Command and display output
- Pass-through argument placement
- Cache defaults
- Serial grouping
- Unfiltered entrypoint selection

### Phase 4: Split the Cargo implementation by responsibility

Extract modules around data and behavior boundaries rather than mechanically
splitting by file size:

```text
cargo/
  mod.rs
  model.rs
  discovery.rs
  tasks.rs
  output.rs
  config.rs
  lockfile.rs
  prune.rs
```

Keep `CargoToolchain` as the integration layer over these focused components.
Move unit tests beside their owning modules. Retain a smaller integration suite
for discovery, execution, filtering, cache restoration, affectedness, watch,
and prune behavior.

### Phase 5: Make cache decisions explicit

Replace output-layout `Option` results with typed decisions that preserve
rejection reasons. Model input trust separately from output predictability. For
example:

```rust
enum OutputLayoutKnowledge {
    Predictable(CargoOutputLayout),
    Unknown(BTreeSet<OutputUncertainty>),
}

enum InputTrust {
    Tracked,
    Untracked(BTreeSet<InputUncertainty>),
}
```

Preserve the current fail-closed behavior exactly. Add debug tracing for the
specific reason automatic caching was disabled. Use table-driven tests for
every unsupported argument, environment variable, manifest feature,
configuration source, target, profile, and path-containment case.

### Phase 6: Revisit the generic toolchain architecture

Do this only after the Rust-local cleanup is stable.

Introduce a neutral package descriptor containing native manifest path,
directory, toolchain ID, role, authored tasks, direct internal edges, and
external dependencies. Migrate Cargo away from synthesized `PackageJson` and
`workspace:*` dependencies incrementally.

Split the broad `Toolchain` trait into explicit capabilities such as:

- Package discovery
- Task runtime
- Task I/O derivation
- Task selection
- Watch integration
- Prune integration
- Compile-cache integration

Capability absence should be explicit instead of represented by default no-op
methods.

Consolidate task entrypoint selection into one post-filter phase using a
structured selection request instead of `prefer_workspace: bool`.

Move sccache startup and shutdown behind a concrete incremental compile-cache
service rather than exposing sccache-specific details through nominally generic
types.

### Phase 7: Reorganize tests and update documentation

Add reusable but transparent Cargo test fixtures and typed dry-run assertions.
Avoid helpers that hide the manifest or task configuration relevant to a test.

Retain end-to-end coverage for:

- Mixed JavaScript and Cargo repositories
- Pure Cargo repositories
- Package and task-level filtering
- Affectedness, including dev-dependency cycles
- Exact output restoration and cache isolation
- Watch rediscovery and target-directory exclusion
- Pruned workspace buildability and Docker layout
- Command overrides
- Compile-cache eligibility, execution, summary, and shutdown

Update `crates/turborepo/ARCHITECTURE.md`, the Rust guide, generated schemas,
and generated TypeScript definitions whenever behavior or public modeling
changes.

## Pull Request Sequence

Keep each step behavior-preserving and independently reviewable:

1. Add lifecycle and Cargo-only characterization tests.
2. Centralize repository and toolchain composition.
3. Publish an immutable Cargo discovery snapshot.
4. Introduce typed Cargo package and task models.
5. Extract Cargo modules and reorganize unit tests.
6. Introduce explicit cache-safety decisions and diagnostics.
7. Add a neutral package descriptor behind compatibility adapters.
8. Split toolchain capabilities and consolidate task selection.
9. Isolate the sccache lifecycle and add end-to-end coverage.
10. Remove compatibility fields and update documentation.

## Verification

Run after each stage:

```text
cargo test -p turborepo-repository
cargo test -p turborepo-lib
cargo test -p turborepo --test cargo_workspace_test
cargo test -p turborepo --test task_command_test
cargo lint
```

Additionally compare representative dry-run and query output before and after
each structural change. Every phase must preserve:

- Task IDs and implicit task availability
- Cargo command lines and pass-through argument placement
- Filter, exclusion, affected, and package-qualified task behavior
- Task hashes and external dependency attribution
- Cache defaults, exact outputs, and fail-closed safety behavior
- Watch rediscovery semantics
- Prune output layout and buildability
- Existing diagnostics and feature-flag gating

The neutral package model and capability split are the highest-churn work. They
should not be combined with the initial correctness and lifecycle refactors.
