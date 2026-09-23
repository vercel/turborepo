# Native scope contract ownership (TURBO-6101)

Go and Cargo scopes can be tested through the production `RepositoryContributor` and graph-construction boundaries without invoking `go`, `cargo`, or the assembled `turbo` binary. Test doubles contribute observed packages and relationships at that boundary; JavaScript co-location uses the existing discovery and manifest-loading seams. Toolchain discovery itself and real command execution remain separate concerns.

## Moved in this PR

| End-to-end Go case removed | In-process replacement in `turborepo-scope/tests/go_scope.rs` |
| --- | --- |
| Co-located JavaScript/Go directory and cwd inference | `filter_by_directory_selects_colocated_scopes`, `colocated_cwd_inference_selects_javascript_and_go_scopes` |
| Package-name filtering | `filter_by_name_selects_only_the_go_module` |
| Affected selection of internal dependents | `affected_go_modules_include_native_dependents` |

`cargo_scope.rs` exercises injected Cargo manifest identities, aggregate scopes, production-edge affectedness, and cycle-closing input edges. `turborepo-repository::cargo::test::metadata_relationships_keep_cycle_closing_dev_inputs_without_task_cycles` interprets in-memory metadata without running Cargo. These Cargo assertions add a narrow contract; this PR does **not** remove Cargo end-to-end cases. Pure Go module discovery and mixed-language package listing also remain end to end until the production Go observation-to-package assembly is tested without toolchain processes in its own PR.

## Retained end-to-end rationale and follow-ups

| Family | Why retain a representative process test |
| --- | --- |
| Native execution and pass-through arguments | Prove actual `go`/`cargo` children receive commands, cwd, environment and arguments. |
| Cache restoration | Build, delete and restore real native outputs on a cache hit. |
| Prune/buildability | Build a pruned workspace with the real toolchain. |
| Watch, formatting and toolchain identity | Exercise OS events, process behavior, toolchain-specific configuration and side effects. |
| Planning/hash/query/output suites still in the binary | **Not** justified as permanent end-to-end coverage. Move Go task-graph/hash/query, Cargo environment/output/query, and Python/uv contracts through PR-sized Linear children before shrinking the matrix further. |

## Counts and timing limits

- Named Go/Cargo end-to-end tests: **42 Go + 67 Cargo = 109 before; 39 Go + 67 Cargo = 106 after** (three shared `common` harness tests are also enumerated by nextest in both states).
- The three removed Go cases contained **seven assembled-`turbo` launches** (five co-location dry-runs, one filtered list, one affected dry-run). The exact number of indirect Go subprocesses is not measured; no direct Go invocation was removed from their bodies.
- A comparable before-run execution timing was not captured. The initial `cargo nextest list -p turbo --test go_workspace_test --test cargo_workspace_test` took 35.33 s wall including compilation; it is *not* a before execution baseline. Report matched warm before/after runtime measurements in the subsequent PR-sized migrations rather than claiming a speedup for this PR.
