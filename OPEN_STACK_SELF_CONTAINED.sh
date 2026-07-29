#!/usr/bin/env bash
# Self-contained stacked PR opener — bodies embedded.
# Usage: bash OPEN_STACK_SELF_CONTAINED.sh
set -euo pipefail
REPO="${REPO:-vercel/turborepo}"
TMPDIR_BODIES="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_BODIES"' EXIT

cat > "$TMPDIR_BODIES/01-5811.md" << 'BODY_01'
## Intent

Turborepo is refactoring toward a multi-language repository architecture: ecosystem integrations contribute immutable repository knowledge, and core owns graph/task/hash/cache/watch/prune/query policy without ecosystem-specific branches.

This is **stack layer 1 / 5** of the JavaScript **external-resolution** migration. Earlier work already builds a shared resolution generation at graph construction and uses it for task hashing and lockfile affectedness. This layer migrates **query surfaces** onto that generation.

## Changes

- Store producer-supplied human names on `ExternalPackageIdentity`
- Lazy reverse index: external identity → internal dependents
- `turbo query` external packages / `internalDependents` read the catalog
- N-API `packages_from_lockfile` reads the JavaScript resolution domain

## Out of scope

Prune keys, global hash, deleting `PackageInfo` resolution fields (later layers).

## Testing

- `cargo test -p turborepo-repository --lib test_lockfile_traversal`
- `cargo test -p turborepo-repository --lib external_dependency_reverse_index_is_lazy_and_cached`
- `cargo clippy -p turborepo-repository -p turborepo-query --all-targets -- -D warnings`

Closes TURBO-5811.
BODY_01

gh pr create --repo "$REPO" --draft \
  --base main \
  --head shew/turbo-5811-migrate-external-package-queries \
  --title "refactor: Migrate external package queries to resolution knowledge" \
  --body-file "$TMPDIR_BODIES/01-5811.md"

cat > "$TMPDIR_BODIES/02-5812.md" << 'BODY_02'
## Intent

Continue the external-resolution migration: prune’s JS lockfile-key union must use the same exact per-package resolution identities as hashing/query, not legacy `PackageInfo::transitive_dependencies`.

**Stack:** layer 2 / 5. Base = layer 1 (query migration).

## Changes

- `PackageGraph::external_package_identities_for_packages`
- Prune lockfile-key union consumes those identities for retained JS workspaces
- Preserve external-peer lockfile expansion and JS/Cargo separation

## Out of scope

Lockfile subgraph redesign, Cargo prune plans, global hash, field deletion.

## Testing

- `cargo test -p turborepo-repository --lib test_lockfile_traversal`
- `cargo test -p turborepo-lib --lib prune`
- `cargo clippy -p turborepo-repository -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5812.
BODY_02

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5811-migrate-external-package-queries \
  --head shew/turbo-5812-migrate-prune-external-closures \
  --title "refactor: Migrate prune lockfile keys to resolution identities" \
  --body-file "$TMPDIR_BODIES/02-5812.md"

cat > "$TMPDIR_BODIES/03-5813.md" << 'BODY_03'
## Intent

Finish migrating resolution **consumers**: root/global external fingerprints and the missing-lockfile global-file fallback must come from resolution state / definition sources, matching task hashing—not `PackageInfo` closures or singleton lockfile object checks.

**Stack:** layer 3 / 5. Base = layer 2 (prune).

## Changes

- Root external hash from resolution fingerprint cache (`//`)
- Unavailable JS resolution → hash definition sources + root `package.json`
- Remove owned singleton lockfile reads from global-hash collection

## Out of scope

Task contracts (Phase 5), deleting legacy fields (next layer).

## Testing

- `cargo test -p turborepo-task-hash --lib`
- `cargo test -p turborepo-repository --lib javascript_resolution_distinguishes_resolved_empty_from_unavailable`
- `cargo clippy -p turborepo-repository -p turborepo-task-hash -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5813.
BODY_03

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5812-migrate-prune-external-closures \
  --head shew/turbo-5813-migrate-global-external-hash-inputs \
  --title "refactor: Migrate global hash inputs to resolution knowledge" \
  --body-file "$TMPDIR_BODIES/03-5813.md"

cat > "$TMPDIR_BODIES/04-5814.md" << 'BODY_04'
## Intent

**Deletion gate** for Phase 3: once every consumer reads resolution knowledge, remove legacy resolution state from `PackageInfo`, deferred closure installation, and fallback rehash helpers so readiness belongs to repository construction.

**Stack:** layer 4 / 5. Base = layer 3 (global hash).

## Changes

- Delete `PackageInfo::{unresolved_external_dependencies,transitive_dependencies,external_deps_hash}`
- Delete deferred closure workers / `ensure_transitive_closures`
- Delete `get_external_deps_hash` fallback rehash helper
- Mark Phase 3 complete in `ARCHITECTURE.md`

## Out of scope

Total `PackageInfo` deletion, script/version payload, declaration-path polish (layer 5).

## Testing

- `cargo test -p turborepo-repository --lib test_lockfile_traversal`
- `cargo test -p turborepo-lib --lib prune`
- `cargo test -p turborepo-task-hash --lib`
- `cargo test -p turborepo-frameworks --lib`
- `cargo clippy -p turborepo-repository -p turborepo-lib -p turborepo-task-hash -p turborepo-frameworks -p turborepo-boundaries -p turborepo-hash --all-targets -- -D warnings`

Closes TURBO-5814.
BODY_04

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5813-migrate-global-external-hash-inputs \
  --head shew/turbo-5814-delete-legacy-external-resolution-state \
  --title "refactor: Delete legacy external-resolution PackageInfo state" \
  --body-file "$TMPDIR_BODIES/04-5814.md"

cat > "$TMPDIR_BODIES/05-5825.md" << 'BODY_05'
## Intent

Final Phase 3 cleanup: ensure external **declaration** consumers only use the authoritative `ExternalDeclarations` projection (from relationship knowledge), and remove leftover resolution-lifecycle / compat commentary from the deleted PackageInfo declaration path.

**Stack:** layer 5 / 5. Base = layer 4 (deletion gate).

## Changes

- Remove unused `ExternalResolutionStatus::Resolving`
- Document that frameworks/boundaries/task hashing use `ExternalDeclarations`
- Clean stale unresolved-declaration compat comments

## Testing

- `cargo test -p turborepo-frameworks --lib`
- `cargo test -p turborepo-boundaries --lib`
- `cargo test -p turborepo-repository --lib declaration_view_preserves_effective_external_declarations`
- `cargo clippy -p turborepo-repository -p turborepo-frameworks -p turborepo-boundaries --all-targets -- -D warnings`

Closes TURBO-5825.
BODY_05

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5814-delete-legacy-external-resolution-state \
  --head shew/turbo-5825-remove-external-declaration-compatibility-paths \
  --title "refactor: Remove external declaration compatibility paths" \
  --body-file "$TMPDIR_BODIES/05-5825.md"

cat > "$TMPDIR_BODIES/06-5816.md" << 'BODY_06'
## Intent

Turborepo’s multi-language architecture treats ecosystem integrations as sources of immutable repository knowledge. After Phase 3 moved external-resolution facts into a shared generation, this PR starts **Phase 4 (native tasks and commands)**.

Today JavaScript script execution and Cargo verb tables live behind behavioral `Toolchain` callbacks (and some direct `PackageJson::scripts` reads). This layer introduces a **parser-neutral native-task catalog** produced once during repository construction, validates it against `RepositoryKnowledge`, and turns JS/Cargo `Toolchain` task methods into **adapters over that catalog** (catalog is the sole authority for availability/authorship/registration/command templates).

Later stack layers migrate engine/turbo-json/executor/query/summary consumers and eventually delete the callbacks.

## Changes

- Add `native_tasks` module: observations, scope states (unknown/unobserved/empty/available), command templates, resolve helpers
- Produce JS tasks from package.json scripts (preserving spans) and Cargo tasks from verb tables at discovery
- Retain `NativeTaskKnowledge` on `PackageGraph` / `PackageTaskContext`
- Adapt `JavaScriptToolchain` and `CargoToolchain` `task_command` / display / defines / authors / registered APIs to the catalog

## Out of scope

- Migrating engine / turbo-json / executor / query / summary consumers (TURBO-5817+)
- Deleting Toolchain task callbacks or direct script reads outside adapters
- Task contracts / hashing / cache (Phase 5)

## Testing

- `cargo test -p turborepo-repository --lib native_tasks`
- `cargo test -p turborepo-repository --lib test_javascript_task_command`
- `cargo test -p turborepo-repository --lib cargo::test::test_cargo_task_commands`
- `cargo test -p turborepo-repository --lib cargo::test::test_cargo_derived_task_io`
- `cargo clippy -p turborepo-repository --all-targets -- -D warnings`

Closes TURBO-5816.
BODY_06

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5825-remove-external-declaration-compatibility-paths \
  --head shew/turbo-5816-produce-immutable-native-task-and-command-knowledge \
  --title "refactor: Produce immutable native task and command knowledge" \
  --body-file "$TMPDIR_BODIES/06-5816.md"

cat > "$TMPDIR_BODIES/07-5817.md" << 'BODY_07'
## Intent

Continue Phase 4 of the multi-language repository architecture: after the native-task catalog exists, migrate **registration / availability / suggestion** consumers onto it so engine missing-task accounting and CLI potential-task listing no longer read `PackageJson::scripts` directly.

**Stack:** Phase 4 layer 2. Base = `shew/turbo-5816-produce-immutable-native-task-and-command-knowledge`.

## Changes

- `Run::get_potential_tasks` enumerates authored+registered catalog tasks
- Engine persistent-task validation / concurrency accounting uses catalog `defines`
- Engine add-all already used `registered_tasks` (now catalog-backed via adapters)

## Out of scope

- turbo.json no-config synthesis (TURBO-5818)
- Task-definition precedence, command planning/execution

## Testing

- `cargo test -p turborepo-engine --lib` (127 passed)
- `cargo clippy -p turborepo-lib -p turborepo-engine --all-targets -- -D warnings`

Closes TURBO-5817.
BODY_07

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5816-produce-immutable-native-task-and-command-knowledge \
  --head shew/turbo-5817-migrate-native-task-registration-and-suggestions \
  --title "refactor: Migrate native task registration and suggestions" \
  --body-file "$TMPDIR_BODIES/07-5817.md"

cat > "$TMPDIR_BODIES/08-5818.md" << 'BODY_08'
## Intent

Continue Phase 4: when Turborepo synthesizes task definitions without an authored turbo.json (or in single-package mode), enumerate native tasks from the **native-task catalog** instead of reading `PackageJson::scripts` directly.

**Stack:** Phase 4 layer 3. Base = `shew/turbo-5817-migrate-native-task-registration-and-suggestions`.

## Changes

- Workspace no-turbo.json synthesis collects catalog executable/authored task names
- Single-package synthesis takes root script names from the catalog (`single_package(root_scripts)`)
- Devtools / package-changes watcher updated to the same API

## Out of scope

Task-definition precedence, executor command planning, persistent validation overhaul (later layers).

## Testing

- `cargo test -p turborepo-turbo-json --lib` (178 passed)
- `cargo clippy -p turborepo-turbo-json -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5818.
BODY_08

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5817-migrate-native-task-registration-and-suggestions \
  --head shew/turbo-5818-migrate-turbo-json-native-task-synthesis \
  --title "refactor: Migrate turbo-json native task synthesis" \
  --body-file "$TMPDIR_BODIES/08-5818.md"

cat > "$TMPDIR_BODIES/09-5819.md" << 'BODY_09'
## Intent

Continue Phase 4 of the multi-language repository architecture: persistent-task validation and recursive-turbo detection must use **effective executability / authored display facts from the native-task catalog**, not live `PackageJson::scripts` reads.

**Stack:** Phase 4 layer 4. Base = `shew/turbo-5818-migrate-turbo-json-native-task-synthesis`.

## Changes

- `task_has_command` / entrypoint selection use catalog `defines`
- Visitor recursive-turbo check uses catalog display/script spans (root-only)
- Persistent dependency/concurrency paths already catalog-backed (refined comments/usage)

## Out of scope

Persistent/interactive semantics, stdin lifecycle, cache contracts.

## Testing

- `cargo test -p turborepo-lib --lib engine::` (10 passed)
- `cargo clippy -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5819.
BODY_09

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5818-migrate-turbo-json-native-task-synthesis \
  --head shew/turbo-5819-migrate-persistent-and-recursive-task-validation \
  --title "refactor: Migrate persistent and recursive task validation" \
  --body-file "$TMPDIR_BODIES/09-5819.md"

cat > "$TMPDIR_BODIES/10-5820.md" << 'BODY_10'
## Intent

Continue Phase 4: the engine task-definition resolver must decide command precedence using **catalog authorship / registration / executability**, not by dispatching through behavioral `Toolchain` callbacks.

Precedence (highest → lowest) stays: package command, root `package#task` command, authored native command, unscoped/per-provider command, synthesized native command.

**Stack:** Phase 4 layer 5. Base = `shew/turbo-5819-migrate-persistent-and-recursive-task-validation`.

## Changes

- `defines` / `registers` / `authors` for definition resolution read the native-task catalog
- Remove toolchain callback dispatch from those precedence checks

## Out of scope

Execution framing, task contracts / derived I/O (TURBO-5789).

## Testing

- `cargo test -p turborepo-engine --lib` (127 passed)
- `cargo clippy -p turborepo-engine --all-targets -- -D warnings`

Closes TURBO-5820.
BODY_10

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5819-migrate-persistent-and-recursive-task-validation \
  --head shew/turbo-5820-migrate-native-task-definition-precedence \
  --title "refactor: Migrate native task definition precedence" \
  --body-file "$TMPDIR_BODIES/10-5820.md"

cat > "$TMPDIR_BODIES/11-5821.md" << 'BODY_11'
## Intent

Continue Phase 4: engine/run execution should plan native commands from the **native-task catalog + TaskArgs**, producing an ecosystem-neutral `TaskCommand` without calling `Toolchain::task_command`.

**Stack:** Phase 4 layer 6. Base = `shew/turbo-5820-migrate-native-task-definition-precedence`.

## Changes

- Add `PackageGraph::resolve_native_task_command` (catalog templates + which + overrides)
- `ToolchainCommandProvider` plans via that API instead of `toolchain.task_command`
- Preserve override / pure-native-root / MFE / compile-cache decoration behavior

## Out of scope

Lazy binary cache polish (TURBO-5822), env/hash contract changes.

## Testing

- `cargo test -p turborepo-task-executor --lib` (18 passed)
- `cargo clippy -p turborepo-repository -p turborepo-task-executor --all-targets -- -D warnings`

Closes TURBO-5821.
BODY_11

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5820-migrate-native-task-definition-precedence \
  --head shew/turbo-5821-migrate-engine-native-command-planning \
  --title "refactor: Migrate engine native command planning" \
  --body-file "$TMPDIR_BODIES/11-5821.md"

cat > "$TMPDIR_BODIES/12-5822.md" << 'BODY_12'
## Intent

Continue Phase 4: the executor should consume concrete catalog-backed command plans **without `Toolchain::task_command` lookup**, with lazy cached program resolution for package-manager and cargo binaries.

**Stack:** Phase 4 layer 7. Base = `shew/turbo-5821-migrate-engine-native-command-planning`.

## Changes

- Executor resolves via catalog `resolve_task_command` + cached `which` results (`OnceLock`)
- Remove unused `MissingToolchain` error for task-command lookup
- Preserve override framing, MFE env decorations, and compile-cache injection boundary

## Out of scope

Environment/hash contract changes, process lifecycle semantics.

## Testing

- `cargo test -p turborepo-task-executor --lib` (18 passed)
- `cargo clippy -p turborepo-task-executor --all-targets -- -D warnings`

Closes TURBO-5822.
BODY_12

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5821-migrate-engine-native-command-planning \
  --head shew/turbo-5822-migrate-executor-native-command-resolution \
  --title "refactor: Migrate executor native command resolution" \
  --body-file "$TMPDIR_BODIES/12-5822.md"

cat > "$TMPDIR_BODIES/13-5823.md" << 'BODY_13'
## Intent

Continue Phase 4: query, devtools, and LSP task views must consume the **native-task catalog / observation vocabulary** instead of direct `PackageJson::scripts` reads—while LSP still maps unsaved package.json buffers through the same observation helper so editor features stay source-accurate.

**Stack:** Phase 4 layer 8. Base = `shew/turbo-5822-migrate-executor-native-command-resolution`.

## Changes

- Query package tasks/script/executes use catalog facts
- Devtools root-task enumeration and task-graph script text use catalog
- LSP completions/index/references use `observation_from_package_json` on buffer payloads
- MFE custom `proxy` detection uses catalog `defines`

## Out of scope

GraphQL schema changes, run-summary command display (TURBO-5824).

## Testing

- `cargo test -p turborepo-query --lib` (9 passed)
- `cargo test -p turborepo-task-executor --lib command::tests` (10 passed)
- `cargo clippy -p turborepo-query -p turborepo-lib -p turborepo-lsp -p turborepo-task-executor --all-targets -- -D warnings`

Closes TURBO-5823.
BODY_13

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5822-migrate-executor-native-command-resolution \
  --head shew/turbo-5823-migrate-native-task-query-devtools-and-lsp-views \
  --title "refactor: Migrate native task query, devtools, and LSP views" \
  --body-file "$TMPDIR_BODIES/13-5823.md"

cat > "$TMPDIR_BODIES/14-5824.md" << 'BODY_14'
## Intent

Final Phase 4 gate: dry-run/run summaries must use the same catalog display facts as execution, and production code must no longer call `Toolchain` task-command callbacks. Those callbacks and their JavaScript/Cargo adapters are deleted.

**Stack:** Phase 4 layer 9. Base = `shew/turbo-5823-migrate-native-task-query-devtools-and-lsp-views`.

## Changes

- Summary `command` text uses catalog `display`
- Engine implicit registration uses catalog `registered`
- Task-access Next.js detection uses observation vocabulary
- Delete `Toolchain::{task_command,task_display_command,authors_task,registered_tasks,registers_task,defines_task}` and JS/Cargo impls
- `ARCHITECTURE.md` marks Phase 4 complete

## Out of scope

`task_defaults` / derived I/O / cache contracts (TURBO-5789), total `PackageInfo` deletion.

## Testing

- `cargo test -p turborepo-engine --lib` (127 passed)
- `cargo test -p turborepo-run-summary --lib` (43 passed)
- `cargo test -p turborepo-repository --lib test_javascript_task_command`
- `cargo test -p turborepo-repository --lib test_cargo_task_commands`
- `cargo clippy … -- -D warnings`

Closes TURBO-5824.
BODY_14

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5823-migrate-native-task-query-devtools-and-lsp-views \
  --head shew/turbo-5824-migrate-command-summaries-and-delete-legacy-task-paths \
  --title "refactor: Migrate command summaries and delete legacy task paths" \
  --body-file "$TMPDIR_BODIES/14-5824.md"

cat > "$TMPDIR_BODIES/15-5826.md" << 'BODY_15'
## Intent

Start Phase 5 (task contracts): produce an immutable **task-contract knowledge** catalog for JavaScript scopes. JS packages observe empty derived-I/O contracts (turbo.json is the whole story). Cargo remains on temporary toolchain callbacks until its Rust port.

**Stack:** Phase 5 layer 1. Base = `shew/turbo-5824-migrate-command-summaries-and-delete-legacy-task-paths`.

## Changes

- New `task_contracts` module (`ScopeTaskContract`, `TaskContractKnowledge`)
- Package graph construction indexes JS scopes (including root `//`) with empty contracts
- `PackageTaskContext::task_contract()` exposes the observation

## Out of scope

Engine composition (TURBO-5827), hashing/cache (TURBO-5828), deleting JS I/O callbacks (TURBO-5829).

## Testing

- `cargo test -p turborepo-repository --lib task_contracts` (2 passed)
- `cargo clippy -p turborepo-repository --all-targets -- -D warnings`

Closes TURBO-5826.
BODY_15

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5824-migrate-command-summaries-and-delete-legacy-task-paths \
  --head shew/turbo-5826-produce-immutable-task-contract-knowledge \
  --title "refactor: Produce immutable task-contract knowledge" \
  --body-file "$TMPDIR_BODIES/15-5826.md"

cat > "$TMPDIR_BODIES/16-5827.md" << 'BODY_16'
## Intent

Continue Phase 5: the engine must compose effective task definitions for **JavaScript packages from foundational task-contract knowledge**, without dispatching through `Toolchain::task_defaults` / `derived_task_io` / `derives_task_io`.

Cargo keeps temporary toolchain-derived I/O until its Rust port.

**Stack:** Phase 5 layer 2. Base = `shew/turbo-5826-produce-immutable-task-contract-knowledge`.

## Changes

- Engine applies JS contract defaults (empty) from `PackageTaskContext::task_contract()`
- Engine skips Toolchain derived-I/O dispatch when the package has a JS contract observation
- Cargo path unchanged

## Testing

- `cargo test -p turborepo-engine --lib` (127 passed)
- `cargo clippy -p turborepo-engine --all-targets -- -D warnings`

Closes TURBO-5827.
BODY_16

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5826-produce-immutable-task-contract-knowledge \
  --head shew/turbo-5827-migrate-engine-task-contract-composition \
  --title "refactor: Migrate engine task-contract composition" \
  --body-file "$TMPDIR_BODIES/16-5827.md"

cat > "$TMPDIR_BODIES/17-5828.md" << 'BODY_17'
## Intent

Continue Phase 5: global hashing must consume **root `engines` from task-contract knowledge**, not a live `PackageJson` read on the root compatibility payload.

**Stack:** Phase 5 layer 3. Base = `shew/turbo-5827-migrate-engine-task-contract-composition`.

## Changes

- Capture root `engines` into `TaskContractKnowledge` at graph construction
- `PackageGraph::root_engines()` exposes them
- `collect_global_file_hash_inputs` / `get_global_hash_inputs` take engines from that knowledge

## Out of scope

Deleting JS `derived_task_io` methods (TURBO-5829), framework-inference relocation beyond engines.

## Testing

- `cargo test -p turborepo-task-hash --lib` (18 passed)
- `cargo clippy -p turborepo-repository -p turborepo-task-hash -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5828.
BODY_17

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5827-migrate-engine-task-contract-composition \
  --head shew/turbo-5828-migrate-hashing-and-cache-to-task-contracts \
  --title "refactor: Migrate hashing engines to task contracts" \
  --body-file "$TMPDIR_BODIES/17-5828.md"

cat > "$TMPDIR_BODIES/18-5829.md" << 'BODY_18'
## Intent

Continue Phase 5 deletion gate: JavaScript packages must not participate in Toolchain task-I/O environment projection, and JS must not override `Toolchain::derived_task_io` (foundational contracts own that answer).

**Stack:** Phase 5 layer 4. Base = `shew/turbo-5828-migrate-hashing-and-cache-to-task-contracts`.

## Changes

- `project_task_io_environment` excludes `ToolchainId::JAVASCRIPT`
- Remove no-op `JavaScriptToolchain::derived_task_io` override
- ARCHITECTURE notes Phase 5 progress

## Remaining Phase 5 follow-ups

Fuller framework/tool-identity contract production and the final Phase 5 completion gate still outstanding under TURBO-5789 if more slices are needed.

## Testing

- `cargo test -p turborepo-lib --lib task_io_context_tests` (4 passed)
- `cargo clippy -p turborepo-repository -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5829.
BODY_18

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5828-migrate-hashing-and-cache-to-task-contracts \
  --head shew/turbo-5829-migrate-dry-run-summary-contracts-and-delete-js-io-callbacks \
  --title "refactor: Exclude JavaScript from toolchain task-I/O dispatch" \
  --body-file "$TMPDIR_BODIES/18-5829.md"

cat > "$TMPDIR_BODIES/19-5830.md" << 'BODY_19'
## Intent

Start Phase 6 (change/watch): produce an immutable **change knowledge** catalog for JavaScript — package ownership, membership triggers (`package.json` / workspace config), and resolution triggers (lockfile) — from repository knowledge + the active package manager.

**Stack:** Phase 6 layer 1. Base = `shew/turbo-5829-migrate-dry-run-summary-contracts-and-delete-js-io-callbacks`.

## Changes

- New `change_knowledge` module (`ChangeKnowledge`)
- Package graph construction produces JS change observations
- `PackageGraph::change_knowledge()` + `to_watch_spec` / `combined_watch_spec` helpers for watcher migration

## Out of scope

Watcher consumer migration (TURBO-5832), deleting JS-only reconstruction paths (TURBO-5831).

## Testing

- `cargo test -p turborepo-repository --lib change_knowledge` (2 passed)
- `cargo clippy -p turborepo-repository --all-targets -- -D warnings`

Closes TURBO-5830.
BODY_19

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5829-migrate-dry-run-summary-contracts-and-delete-js-io-callbacks \
  --head shew/turbo-5830-produce-immutable-change-knowledge \
  --title "refactor: Produce immutable change knowledge" \
  --body-file "$TMPDIR_BODIES/19-5830.md"

cat > "$TMPDIR_BODIES/20-5832.md" << 'BODY_20'
## Intent

Continue Phase 6: the package-changes watcher must classify filesystem events using **`PackageGraph::active_watch_spec()`**, which combines foundational change knowledge with active toolchain WatchSpecs — not a raw `toolchains().watch_spec()` merge that ignores JS change knowledge.

**Stack:** Phase 6 layer 2. Base = `shew/turbo-5830-produce-immutable-change-knowledge`.

## Changes

- `active_watch_spec` merges `ChangeKnowledge::to_watch_spec()` with active toolchain specs
- Watcher poll loop uses `active_watch_spec()` for classification (same as rediscovery reinit)
- JS workspace-config paths project into rediscovery triggers; per-package/lockfile facts stay on change knowledge for ChangeMapper granularity

## Testing

- `cargo test -p turborepo-repository --lib change_knowledge` (3 passed)
- `cargo test -p turborepo-lib --lib package_changes_watcher` (43 passed)
- `cargo clippy -p turborepo-repository -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5832.
BODY_20

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5830-produce-immutable-change-knowledge \
  --head shew/turbo-5832-migrate-watcher-classification-to-change-knowledge \
  --title "refactor: Migrate watcher classification to change knowledge" \
  --body-file "$TMPDIR_BODIES/20-5832.md"

cat > "$TMPDIR_BODIES/21-5831.md" << 'BODY_21'
## Intent

Finish Phase 6 first wave: delete JavaScript package-manager lockfile probes from change classification in favor of **foundational change-knowledge `resolution_paths`**.

**Stack:** Phase 6 layer 3. Base = `shew/turbo-5832-migrate-watcher-classification-to-change-knowledge`.

## Changes

- `ScopeChangeDetector::get_lockfile_contents` reads resolution paths from `PackageGraph::change_knowledge()`
- ARCHITECTURE.md marks Phase 6 first-wave completion

## Testing

- `cargo test -p turborepo-scope --lib` (100 passed)
- `cargo test -p turborepo-lib --lib package_changes_watcher` (43 passed)
- `cargo clippy -p turborepo-scope -p turborepo-lib -p turborepo-repository --all-targets -- -D warnings`

Closes TURBO-5831.
BODY_21

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5832-migrate-watcher-classification-to-change-knowledge \
  --head shew/turbo-5831-delete-js-only-watcher-reconstruction-paths \
  --title "refactor: Delete JS lockfile probes from change classification" \
  --body-file "$TMPDIR_BODIES/21-5831.md"

cat > "$TMPDIR_BODIES/22-5833.md" << 'BODY_22'
## Intent

Start Phase 7 (prune) pre-work: extract current JavaScript prune rendering helpers into pure functions without behavior changes, separating them from filesystem orchestration in `commands/prune.rs`.

**Stack:** Phase 7 layer 1. Base = `shew/turbo-5831-delete-js-only-watcher-reconstruction-paths`.

## Changes

- New `commands/prune_js.rs` with manifest/workspace/patch/bin helpers
- `prune.rs` orchestration calls the extracted helpers

## Out of scope

Rewiring onto knowledge-only inputs; golden fixture expansion.

## Testing

- `cargo test -p turborepo-lib --lib commands::prune` (15 passed)
- `cargo clippy -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5833.
BODY_22

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5831-delete-js-only-watcher-reconstruction-paths \
  --head shew/turbo-5833-extract-javascript-prune-rendering-pure-functions \
  --title "refactor: Extract JavaScript prune rendering pure functions" \
  --body-file "$TMPDIR_BODIES/22-5833.md"

cat > "$TMPDIR_BODIES/23-5834.md" << 'BODY_23'
## Intent

Continue Phase 7: core prune orchestration must treat JavaScript lockfile/manifest/patch rewriting as a **distinct rendering step** with explicit inputs/outputs, separate from package-closure selection and path-safe layout.

**Stack:** Phase 7 layer 2. Base = `shew/turbo-5833-extract-javascript-prune-rendering-pure-functions`.

## Changes

- `render_javascript_prune` + `JavaScriptPruneRenderInput` / `JavaScriptPruneRenderResult` in `prune_js.rs`
- `prune.rs` selects closures/layout first, then materializes the rendered JS artifacts

## Out of scope

Golden fixtures; deleting remaining JS compatibility projection.

## Testing

- `cargo test -p turborepo-lib --lib commands::prune` (15 passed)
- `cargo clippy -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5834.
BODY_23

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5833-extract-javascript-prune-rendering-pure-functions \
  --head shew/turbo-5834-separate-prune-closure-and-layout-from-js-rendering \
  --title "refactor: Separate prune closure and layout from JS rendering" \
  --body-file "$TMPDIR_BODIES/23-5834.md"

cat > "$TMPDIR_BODIES/24-5835.md" << 'BODY_24'
## Intent

Continue Phase 7: add golden prune fixtures that compare retained packages, relative file set, content fingerprints, permissions class, and standard/Docker layer placement for the separated render + layout path.

**Stack:** Phase 7 layer 3. Base = `shew/turbo-5834-separate-prune-closure-and-layout-from-js-rendering`.

## Changes

- `inventory_tree` / retained-package helpers in `prune_test.rs`
- Golden snapshots for standard and `--docker` prune of `monorepo_with_root_dep`

## Testing

- `cargo test -p turbo --test prune_test golden_inventory` (2 passed)

Closes TURBO-5835.
BODY_24

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5834-separate-prune-closure-and-layout-from-js-rendering \
  --head shew/turbo-5835-add-prune-golden-fixtures-for-retained-files-and-layers \
  --title "test: Add prune golden fixtures for retained files and layers" \
  --body-file "$TMPDIR_BODIES/24-5835.md"

cat > "$TMPDIR_BODIES/25-5836.md" << 'BODY_25'
## Intent

Finish Phase 7: `commands/prune.rs` must only select closures, perform path-safe layout, and materialize typed `JavaScriptPruneRenderResult` artifacts — with no inline JS lockfile/manifest/patch format interpretation.

**Stack:** Phase 7 layer 4. Base = `shew/turbo-5835-add-prune-golden-fixtures-for-retained-files-and-layers`.

## Changes

- `materialize_javascript_render` writes render artifacts without format logic
- Workspace patch-config path comes from the render result (not a pnpm branch in orchestration)
- ARCHITECTURE.md marks Phase 7 complete

## Testing

- `cargo test -p turborepo-lib --lib commands::prune` (15 passed)
- `cargo test -p turbo --test prune_test golden_inventory` (2 passed)
- `cargo clippy -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5836.
BODY_25

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5835-add-prune-golden-fixtures-for-retained-files-and-layers \
  --head shew/turbo-5836-delete-js-format-interpretation-from-prune-orchestration \
  --title "refactor: Delete JS format interpretation from prune orchestration" \
  --body-file "$TMPDIR_BODIES/25-5836.md"

cat > "$TMPDIR_BODIES/26-5837.md" << 'BODY_26'
## Intent

Phase 8 audit: confirm query/summary/run/engine/devtools/watch/prune consumers are views over shared knowledge, migrate any owned leftover script reads, and explicitly park remaining `PackageJson`/`PackageInfo` construction/compat reads under TURBO-5787.

**Stack:** Phase 8 layer 1. Base = `shew/turbo-5836-delete-js-format-interpretation-from-prune-orchestration`.

## Changes

- Devtools package-graph scripts list reads native-task catalog instead of `PackageJson::scripts`
- ARCHITECTURE.md Phase 8 audit status + inventory of remaining compat reads for TURBO-5787

## Remaining (TURBO-5787)

- `PackageInfo` / payload map deletion
- MFE `all_dependencies` via compatibility payload
- Prune peer/optional-peer helpers via `PackageJson`
- Graph construction entry `PackageJson::load` sites
- LSP unsaved-buffer `PackageJson` adapter (intentionally retained)
- Full `JavaScriptToolchain` removal

## Testing

- `cargo check -p turborepo-devtools`
- `cargo clippy -p turborepo-devtools -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5837.
BODY_26

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5836-delete-js-format-interpretation-from-prune-orchestration \
  --head shew/turbo-5837-audit-and-close-remaining-js-knowledge-consumer-reads \
  --title "refactor: Audit and close remaining JS knowledge consumer reads" \
  --body-file "$TMPDIR_BODIES/26-5837.md"

cat > "$TMPDIR_BODIES/27-5838.md" << 'BODY_27'
## Intent

Start TURBO-5787 (JS compatibility removal): microfrontends `@vercel/microfrontends` dependency detection must use **external-declaration knowledge**, not `PackageInfo::package_json.all_dependencies()`.

**Stack:** Compatibility-removal layer 1. Base = `shew/turbo-5837-audit-and-close-remaining-js-knowledge-consumer-reads`.

## Changes

- `has_mfe_dependency` reads `PackageTaskContext::external_declarations`
- `javascript_packages` no longer yields `PackageInfo`

## Out of scope

PackageInfo deletion, JavaScriptToolchain deletion, prune peer helpers.

## Testing

- `cargo test -p turborepo-lib --lib microfrontends` (24 passed)
- `cargo clippy -p turborepo-lib --all-targets -- -D warnings`

Closes TURBO-5838.
BODY_27

gh pr create --repo "$REPO" --draft \
  --base shew/turbo-5837-audit-and-close-remaining-js-knowledge-consumer-reads \
  --head shew/turbo-5838-migrate-mfe-dependency-detection-off-packageinfo \
  --title "refactor: Migrate MFE dependency detection off PackageInfo" \
  --body-file "$TMPDIR_BODIES/27-5838.md"

echo "Opened 27-layer stack through tip."
