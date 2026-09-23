# Go and Cargo test ownership (TURBO-6101)

A native invocation belongs in `crates/turborepo/tests/{go,cargo}_workspace_test.rs` only when it checks behavior of the actual toolchain or the assembled CLI. Planning observations belong to the crate that owns them: `turborepo-repository` (contribution, metadata, relationships, native tasks, environment and output contracts), `turborepo-scope` (filtering/affectedness), then the task-hash, task-graph, query, or prune crate for their respective semantics. Inject observed packages at `RepositoryContributor`, JavaScript discovery at `with_package_discovery`, and manifests at `with_package_json_loader`; do not launch `go`, `cargo`, or `turbo` to prove these planning contracts.

## Moved in this change

| Removed end-to-end scenario | Replacement / owning boundary |
| --- | --- |
| Pure Go module list; mixed JS + Go module list | In-memory Go contribution and JS discovery/manifest injection in `turborepo-repository::go::tests`; Go scope identity in `turborepo-scope/tests/go_scope.rs` |
| Co-located JS/Go cwd inference; Go name filter | In-memory contributor and `FilterResolver`/`PackageInference` in `turborepo-scope/tests/go_scope.rs` |
| Go `--affected` internal dependent selection | In-memory native relationships and injected change detector in `turborepo-scope/tests/go_scope.rs` |
| Cargo implicit task registration without `turbo.json` | Cargo's in-memory native task/context command tests and registered-task table in `turborepo-repository::cargo::test` |
| Cargo compiler/layout environment disables inferred caching | Cargo output-layout fail-closed table in `turborepo-repository::cargo::test` |
| Cargo `test`/`bench` profile directory combinations | Cargo output-argument profile table in `turborepo-repository::cargo::test`; custom profile restoration still covered end to end |
| Cargo CLI target overriding environment target | Cargo output-layout precedence in `turborepo-repository::cargo::test`; actual target-directory precedence/restoration remains end to end |

The Go contributor's observation-to-package assembly is separated from its native subprocess observations, so in-memory graph, external-resolution, watch, and prune tests exercise the *production* contribution code. The Cargo metadata-to-relationship test also builds a dev-dependency cycle entirely in memory, checking that its back edge contributes inputs without ordering tasks. Cargo scope identity and production-edge affectedness are independently checked through an injected contributor in `turborepo-scope/tests/cargo_scope.rs`; task-input affectedness for cycle-closing dev edges remains a separate end-to-end query check.

## Retained end-to-end rationale

| Family | Why a real process is required | Representative retained tests |
| --- | --- | --- |
| Execution and forwarding | A real `go`/`cargo` child must receive the right cwd, environment, subcommand and arguments. | Go `test_native_go_tasks_execute_cache_restore_and_pass_through_args`; Cargo `test_cargo_debug_and_release_caches_are_isolated_both_directions` (`--release` forwarded), `test_cargo_build_executes_caches_and_restores` |
| Cache restoration | Build a deliverable, delete it, hit the CLI cache, and confirm the native output is restored. | Go `test_native_go_tasks_execute_cache_restore_and_pass_through_args`; Cargo `test_cargo_build_executes_caches_and_restores` |
| Prune/buildability | Actual `go`/`cargo` must build the pruned workspace; a manifest-only check cannot prove that. | Go `test_go_prune_produces_minimal_valid_workspace`; Cargo `test_prune_produces_buildable_cargo_workspace` |
| Process/toolchain behavior | Watcher lifecycle, formatter effects, rustup selection, Go version/CGO or platform-specific paths cannot be established with graph fixtures alone. | Go watch/format/version tests; Cargo rustup/format tests |
| Cross-layer CLI checks still awaiting migration | Hash, query, dry-run, command override, graph and output *planning* should ultimately be tested at their owning crate boundary. These are **not** claimed as necessary E2E coverage; they remain until equivalent injectable task-graph/hash/query/prune coverage exists. | Other tests in the two integration targets |

## Counts and timings

- Before: 42 Go + 67 Cargo named integration scenarios = 109. After: 37 Go + 63 Cargo = 100 (plus three shared `common` harness tests enumerated by nextest in both states). Narrow planning coverage added: 5 Go contributor tests, 6 Go scope tests, 1 Cargo metadata relationship test, and 2 Cargo scope tests; existing Cargo command/output tests cover the other removed cases.
- Statically removed: **34 assembled-`turbo` launches** (9 Go, 25 Cargo) across nine tests. Each launch would also discover the native workspace; the exact number of indirect `go`/`cargo` subprocesses is not measured. No direct native toolchain calls were removed from those nine test bodies.
- Before test-run timing was not captured before changing these tests. The initial `cargo nextest list -p turbo --test go_workspace_test --test cargo_workspace_test` took **35.33 s wall including compilation**; this is inventory/build time, **not** a before execution baseline. After, five representative retained integration tests took **9.256 s nextest run time** (17.83 s wall including rebuilding) on the final integration-test source. These numbers are not comparable as a speedup claim. Collect matched warm before/after runs for a reliable runtime comparison.

This migration is incremental: the remaining pure hash/query/dry-run/output scenarios still require owning-crate injection tests before the end-to-end matrix can be reduced to a small representative set.
