# Dependency-Driven Native Tasks

## Summary

Extend Native Tasks so an ecosystem contributor can discover repository tooling and emit:

- Qualified executable tasks, such as `lint:ruff` and `check:mypy`.
- Canonical aggregate tasks, such as `lint` and `check`, that fan out to qualified tasks.
- Canonical single-provider tasks, such as `format`, that select one qualified implementation.

Python with uv is the first consumer. The core design must also support a later JavaScript implementation without teaching core about individual tools or task categories.

The implementation should generalize task orchestration, command construction, and task contracts in core. Dependency parsing, tool recognition, command policy, root inheritance, formatter precedence, and warnings remain ecosystem-owned.

## Goals

- Detect supported Python tools from direct `pyproject.toml` dependency declarations.
- Run every detected linter and type checker through canonical tasks.
- Run exactly one detected formatter through the canonical `format` task.
- Expose qualified tasks so users can run or configure an individual tool.
- Treat root Python tooling as a workspace default that members can override by role.
- Preserve filtering, entrypoint selection, task graph visibility, overrides, hashing, and logs.
- Reuse the same core model for dependency-driven JavaScript tasks later.
- Keep discovery independent of `.venv` state and the installed `uv` binary.

## Non-Goals

- Discover arbitrary console scripts from installed environments.
- Infer capabilities from transitive dependencies.
- Add JavaScript tool detection in this change.
- Build a public plugin API or a universal cross-ecosystem tool registry.
- Compose commands through a shell.
- Cache dynamically discovered Python quality tasks in the initial implementation.
- Automatically run multiple formatters.

## Product Semantics

### Task Names

The initial Python registry emits these qualified tasks:

| Dependency | Qualified task | Role |
| --- | --- | --- |
| `ruff` | `lint:ruff` | Linter |
| `ruff` | `format:ruff` | Formatter |
| `black` | `format:black` | Formatter |
| `mypy` | `check:mypy` | Type checker |
| `ty` | `check:ty` | Type checker |
| `pyright` | `check:pyright` | Type checker |

The registry must be declarative and easy to extend with additional known tools. Distribution names and executable names must be represented separately because they are not universally identical.

### Canonical Tasks

- `lint` is an aggregate that depends on every effective qualified linter task.
- `check` is an aggregate that depends on every effective qualified type-checker task.
- `format` selects one effective formatter using fixed precedence.
- Qualified tasks remain directly runnable even when they are not selected by a canonical task.

If no recognized formatter is declared, retain the existing `uv format` behavior. If no recognized type checker is declared, retain the existing `uv check` behavior. There is no built-in fallback for `lint`.

### Formatter Precedence

The initial formatter precedence is:

1. Ruff
2. Black

When multiple formatters are effective for a scope, select the highest-precedence formatter and emit one deterministic warning. The warning must identify:

- The affected package or workspace.
- Every detected formatter.
- The selected formatter.
- The precedence that caused the selection.
- The qualified task that runs each non-selected formatter.

Example:

```text
Detected multiple Python formatters for py-app: ruff, black. Native Tasks will use ruff for `format` because formatter precedence is ruff > black. Run `format:black` explicitly to use Black.
```

### Root Tooling

Root tooling is a workspace default.

- A root declaration applies to every member that does not declare tooling for the same role.
- A member declaration overrides root tooling for that role.
- A member override does not affect other roles. For example, member-local Black can override root Ruff formatting while retaining root Ruff linting.
- If every member has the same effective plan, an unfiltered run may use the synthetic workspace package once.
- If effective plans differ, omit the workspace aggregate entrypoint and allow package candidates to execute independently.

Keeping heterogeneous execution package-local avoids cross-package implicit task dependencies and preserves filtering semantics.

### Arguments

Qualified executable tasks accept pass-through arguments.

```bash
turbo run check:mypy -- --strict
```

Canonical aggregates with multiple children reject pass-through arguments with a diagnostic that directs the user to a qualified task. The arguments cannot safely be forwarded to tools with unrelated CLIs.

Canonical `format`, and any canonical task with one direct implementation, may pass arguments to that implementation.

### Overrides And Exclusions

- A `command` argv override replaces native execution and disables native implicit dependencies for that task.
- A `command` opt-out disables both native execution and native implicit dependencies.
- `extends: false` with no other configuration excludes a registered native task.
- `extends: false` with additional configuration reintroduces the task using the existing inheritance semantics.
- An override on a qualified child replaces only that child command; its canonical parent can continue to reference it.
- An opt-out on a qualified child makes the child a no-op. The parent edge may remain.
- Authored JavaScript scripts will take precedence over later inferred canonical JavaScript tasks. The JavaScript producer will resolve those collisions before contributing observations.

## Core Refactor

### 1. Model Native Task Execution Explicitly

Refactor `NativeTask` in `crates/turborepo-repository/src/native_tasks.rs` so task availability is not synonymous with having one executable process.

Conceptual API:

```rust
pub enum NativeTaskExecution {
    Command(NativeCommandTemplate),
    Aggregate {
        dependencies: Vec<NativeTaskDependency>,
    },
    None,
}

pub struct NativeTaskDependency {
    task: String,
}
```

Initially, native dependencies are same-scope task names. This is sufficient for Python and the expected JavaScript model.

Replace ambiguous boolean usage with explicit queries such as:

- `registered()`
- `authored()`
- `has_command()`
- `is_aggregate()`
- `participates()`
- `dependencies()`

Audit existing uses of `defines()` and `executable()`. Graph selection, task queries, devtools, and missing-task checks must recognize aggregates as participating tasks, while the executor must only create a process for command tasks.

Empty authored scripts remain `NativeTaskExecution::None` and preserve their current behavior.

### 2. Compose Native Dependencies In The Engine

In `crates/turborepo-engine/src/builder/definitions.rs`, apply native aggregate dependencies after turbo.json inheritance and command override resolution but before task graph traversal.

- Append native dependencies to `TaskDefinition.task_dependencies`.
- Preserve explicit user `dependsOn` entries.
- Sort and deduplicate the merged dependencies.
- Apply native dependencies only when native fallback behavior remains active.
- Let the existing engine graph validator detect self-dependencies and cycles.

Do not make the Python producer construct engine `TaskDefinition` values directly. Contributors emit immutable native task observations; core owns their composition with user configuration.

Update task-definition memoization so definitions with different native task contracts or dependencies cannot share an invalid memo entry. A stable native-task fingerprint or disabling memoization for these tasks are both acceptable; prefer the smallest correct change.

### 3. Move Task-Local Contract Facts Onto Native Tasks

The current `DynamicTaskContract::{Cargo, Python}` enum in `crates/turborepo-repository/src/task_contracts.rs` is a closed dispatch point that would otherwise require a JavaScript variant.

Introduce parser-neutral task-local contract data associated with each `NativeTask`:

```rust
pub struct NativeTaskContract {
    defaults: TaskDefaults,
    entrypoint: Option<TaskEntrypoint>,
    io: Option<NativeTaskIoSpec>,
}

pub enum AutomaticInputPolicy {
    None,
    PackageSources,
    PackageAndDependencySources,
    PrecomputedGlobs,
}
```

The exact representation may differ, but it must support:

- Default cache policy.
- Entrypoint classification.
- Package source inputs.
- Internal dependency source closures for type checking.
- Precomputed workspace and configuration globs.
- Outputs becoming unavailable when pass-through arguments can relocate them.

Keep scope-wide concerns in `ScopeTaskContract`:

- Toolchain identity.
- Command-map target.
- Entrypoint domain.
- Environment domain and variables.
- Prune package mode.
- Whether the scope participates in dependent source inputs.
- Execution-only scope decorations.

Do not force Cargo into a fully declarative contract in the same change if that increases risk. It is acceptable to retain temporary Cargo-specific dynamic output behavior while moving Python quality tasks to neutral task-local plans. Do not add more Python tool dispatch or a JavaScript variant to the closed dynamic enum.

### 4. Generalize Native Command Construction

The existing `NativeCommandTemplate` variants encode ecosystem-specific argument behavior. Refactor them into a narrow launcher plus argument layout rather than adding variants for Ruff, Black, mypy, ty, or Pyright.

The command model must support:

- A resolved toolchain executable, such as `cargo` or `uv`.
- Package-manager script execution.
- Fixed arguments before pass-through arguments.
- Fixed arguments after pass-through arguments.
- An optional separator before pass-through arguments.
- Working-directory policy.
- Serial execution group.
- Tool-specific missing-binary diagnostics.

This must represent:

```text
cargo test --package=foo --locked -- <args>
uv build --package=foo <args>
uv run --frozen ruff check <args> packages/foo
```

Keep package-manager script execution specialized because npm, pnpm, Yarn, and Bun have different invocation and separator behavior. Add package-manager executable support only when the later JavaScript design establishes its exact semantics.

Replace executor plumbing that passes separate optional package-manager, Cargo, and uv binaries with a resolver abstraction or equivalent data-driven lookup if doing so materially simplifies the command model. Do not build a public executable plugin system.

### 5. Validate Aggregate Arguments

Add parser-neutral validation for pass-through arguments supplied to aggregate tasks.

- Reject arguments for aggregates with multiple children.
- Name the aggregate and its qualified children in the diagnostic.
- Do not change argument propagation for ordinary turbo.json `dependsOn` edges.
- Ensure execution and task hashing use the same effective argument source.

Avoid general graph argument forwarding until a concrete use case has compatible child CLIs.

## Python Implementation

### Dependency Discovery

Extend `crates/turborepo-repository/src/uv.rs` to retain recognized direct dependency declarations while parsing root and member manifests.

Recognize declarations from:

- `[project].dependencies`
- `[dependency-groups]`
- Legacy `[tool.uv].dev-dependencies`

Retain declaration origin so `uv run` can activate a non-default dependency group explicitly when required. Resolve nested `include-group` entries with cycle protection.

For the first iteration:

- Use direct dependencies only.
- Normalize distribution names with the existing PEP 503 normalization.
- Exclude optional-extra-only declarations.
- Exclude marker-qualified tool declarations unless marker evaluation is implemented correctly.
- Warn or document why excluded declarations do not create tasks.
- Do not inspect `.venv`, wheel metadata, or transitive `uv.lock` entries to discover commands.

### Tool Registry

Keep a Python-owned declarative registry containing:

- Distribution name.
- Executable name.
- Role.
- Qualified task name.
- Fixed command arguments.
- Input profile.
- Known configuration files.
- Formatter precedence when applicable.

Core receives only emitted task facts and does not receive Python role or tool enums.

### Commands

Run declared tools through `uv run --frozen` so task execution cannot update `uv.lock`.

Examples:

```text
uv run --frozen ruff check packages/py-app
uv run --frozen ruff format packages/py-app
uv run --frozen black packages/py-app
uv run --frozen mypy packages/py-app
uv run --frozen ty check packages/py-app
uv run --frozen pyright packages/py-app
```

Commands must activate the declaration's dependency group when it is not already active.

All initial `uv run` tasks use the `uv` serial group because they may synchronize the shared environment. This can be relaxed later if environment preparation and tool execution become separable.

### Native Task Plans

For each effective scope:

1. Compute member-local tools by role.
2. Fill missing roles from root tooling.
3. Emit every effective qualified executable task.
4. Emit aggregate `lint` and `check` tasks when qualified children exist.
5. Emit canonical `format` using formatter precedence.
6. Emit the existing uv fallback for `format` or `check` when no recognized provider exists.
7. Assign entrypoint classifications so homogeneous unfiltered runs prefer the workspace aggregate and heterogeneous runs use package candidates.

Use one plan as the source of truth for registration, display strings, command execution, task dependencies, and task-local contracts. Do not maintain separate task-name tables that can drift.

### Hashing And I/O

Keep dynamic quality tasks uncached initially.

Derived inputs include:

- Package sources for all quality tasks.
- Internal dependency source closures for type checkers.
- All member sources for workspace aggregate quality tasks.
- Root and member `pyproject.toml` files as applicable.
- `uv.toml` and `.python-version`.
- Existing relevant uv and pip environment variables.
- Tool-specific configuration files.

Initial known configuration files include:

- Ruff: `pyproject.toml`, `ruff.toml`, `.ruff.toml`
- Black: `pyproject.toml`
- mypy: `pyproject.toml`, `mypy.ini`, `.mypy.ini`, `setup.cfg`
- ty: `pyproject.toml`, `ty.toml`
- Pyright: `pyproject.toml`, `pyrightconfig.json`

Exclude tool caches and generated environments from automatic inputs and watch feedback where applicable, including `.ruff_cache`, `.mypy_cache`, `.pyright`, `.venv`, and Python bytecode caches.

Root-inherited filtered tasks may execute a tool whose version is represented by the root external dependency closure rather than the member closure. Until cross-scope external fingerprints are supported, conservatively include `uv.lock` as an input for those tasks and leave them uncached.

## JavaScript Compatibility

The later JavaScript producer should be able to reuse the core work without additional task-graph concepts.

It will own:

- Detection from `dependencies`, `devDependencies`, and package-manager state.
- Root tooling defaults and package overrides.
- ESLint, Biome, Prettier, TypeScript, and other registry entries.
- Authored-script precedence.
- Package-manager executable invocation.
- JavaScript formatter precedence and warnings.

Do not add methods such as `detect_linters()` to `PackageJson`. Keep `package_json.rs` as manifest data and place detection in the JavaScript contributor or a dedicated JavaScript native-task producer.

Extract a shared tool-registry abstraction only after Python and JavaScript reveal genuinely identical behavior. The core aggregate task model is the intentional shared abstraction for now.

## Testing Plan

### Core Unit Tests

- Command native tasks retain current JavaScript, Cargo, and uv behavior.
- Aggregate native tasks are selectable without a command.
- Aggregate dependencies become same-scope task graph edges.
- Native and explicit dependencies merge and deduplicate.
- Native dependency cycles and self-dependencies use existing diagnostics.
- Command overrides and opt-outs disable native dependencies.
- `extends: false` excludes registered aggregates.
- Empty authored scripts remain non-executable and non-aggregate.
- Query, devtools, run discovery, and missing-task checks recognize aggregates correctly.
- Executor never attempts to launch an aggregate.
- Multi-child aggregates reject pass-through arguments.
- Task-definition memoization cannot cross native contract differences.

### Python Unit Tests

- PEP 503 name normalization.
- Production, dependency-group, and legacy dev dependency recognition.
- Nested dependency-group includes and cycles.
- Optional and marker-qualified declaration handling.
- Root defaults by role.
- Member overrides by role.
- Multiple linters and type checkers.
- Ruff and Black formatter precedence.
- Deterministic, deduplicated formatter warnings.
- Exact command prefix, suffix, targets, group activation, and display strings.
- Qualified and canonical task contracts.
- Workspace aggregate eligibility for homogeneous and heterogeneous plans.

### Integration Tests

Extend `crates/turborepo/tests/uv_workspace_test.rs` and uv fixtures to cover:

- Root-only tooling.
- Member-only tooling.
- Root defaults with a member override.
- `lint` running all detected linters.
- `check` running all detected type checkers.
- Qualified-task filtering.
- Formatter precedence and warning output.
- Explicit non-selected formatter execution.
- Existing uv fallbacks when no supported tools are declared.
- Filtered and unfiltered homogeneous execution.
- Heterogeneous plans falling back to package candidates.
- `uv.lock` remaining unchanged after execution.
- Tool failures propagating independently.
- Mixed JavaScript and Python repositories.
- Dry-run and query output showing canonical and qualified tasks.
- Watch mode ignoring tool cache writes.

Pin tool versions in committed fixture lockfiles. Execution tests should skip only when `uv` itself is unavailable, matching existing uv integration-test behavior.

## Documentation

Update:

- `crates/turborepo/ARCHITECTURE.md` for aggregate Native Tasks, implicit native dependencies, task-local contracts, command construction, and Python tool discovery.
- `apps/docs/content/docs/guides/tools/python.mdx` for detected tools, qualified tasks, canonical behavior, root defaults, formatter precedence, warnings, arguments, caching, and limitations.

Documentation must make clear that:

- Detection is based on supported direct dependency declarations, not arbitrary installed executables.
- Multiple linters and type checkers run, but only one formatter is selected.
- Qualified tasks provide explicit control.
- Dynamic quality tasks are initially uncached.
- uv remains the only supported Python package manager.

## Implementation Sequence

1. Refactor `NativeTask` into command, aggregate, and none execution modes.
2. Audit task-presence and executable-command consumers.
3. Compose native dependencies in the engine with override and exclusion semantics.
4. Add aggregate argument validation.
5. Add task-local neutral contract data and migrate Python quality-task contracts.
6. Generalize native command argument placement.
7. Add Python dependency retention and the initial tool registry.
8. Implement root defaults, member overrides, canonical fan-out, and formatter precedence.
9. Add hashing, watch exclusions, warnings, and uv fallback behavior.
10. Add unit and integration coverage.
11. Update architecture and user documentation.

Each stage should leave existing JavaScript, Cargo, and uv behavior passing before the next stage begins.

## Acceptance Criteria

- `turbo run lint` runs every detected effective Python linter.
- `turbo run check` runs every detected effective Python type checker.
- `turbo run format` runs one formatter using documented precedence.
- Multiple formatter declarations produce a clear deterministic warning.
- Qualified tasks run individual tools and accept pass-through arguments.
- Root tools apply as defaults and member tools override by role.
- Canonical aggregate tasks appear correctly in dry runs, queries, and the task graph.
- User command overrides and task exclusions remain authoritative.
- No shell composition or environment inspection is used.
- Dynamic quality tasks do not update `uv.lock` and remain uncached initially.
- The core implementation contains no Python or JavaScript tool names or formatter precedence policy.
- A future JavaScript producer can emit the same command and aggregate task facts without another task-graph refactor.
