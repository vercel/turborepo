# turborepo-trace-core

The data model, scheduler, file system cache, graph types and outputs shared by the
`turbo trace` crates (`crates/turborepo-trace*`).

## Debug switches

`TURBO_TRACE_DEBUG_<NAME>` environment variables switch one rule of the selection off for an
A/B measurement, turn on timings and dumps, or point a cache elsewhere. They are not a user
interface: not documented for users, not configuration, and free to change between versions.
Options of `turbo trace` are flags.

They exist only with the cargo feature `debug-knobs` of this crate (or of `turborepo-trace`,
which forwards it). It is off by default, so a release build has none of them: every switch
reads as unset, which is its default. The trace crates' tests and `turbo`'s `trace_test` turn
it on through their dev-dependencies. To build `turbo` with the switches:

```sh
cargo build -p turbo -p turborepo-trace-core --features turborepo-trace-core/debug-knobs
```

All of them are read through `turborepo_trace_core::debug`. A rule switch is on unless set to
`0`; the code that reads a switch documents its values.

| Variable | Read in |
|---|---|
| `TURBO_TRACE_DEBUG_ANCHORS` | `turborepo-trace-lang-go/lib.rs`, `turborepo-trace-lang-js/lib.rs` |
| `TURBO_TRACE_DEBUG_ANCHORS_OFF` | `turborepo-trace-lang-js/names.rs` |
| `TURBO_TRACE_DEBUG_ATTR_DUMP` | `turborepo-trace/attrdump.rs` |
| `TURBO_TRACE_DEBUG_BARREL_TIMING` | `turborepo-trace-graph/barrel.rs` |
| `TURBO_TRACE_DEBUG_BARRELS` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_BIND` | `turborepo-trace/xfile.rs` |
| `TURBO_TRACE_DEBUG_BIND_DEBUG` | `turborepo-trace/xfile.rs` |
| `TURBO_TRACE_DEBUG_BIND_FACTS` | `turborepo-trace/xfile.rs` |
| `TURBO_TRACE_DEBUG_BIND_NAMES` | `turborepo-trace/xfile.rs` |
| `TURBO_TRACE_DEBUG_BIND_ROUNDS` | `turborepo-trace-lang-js/xfile/bind.rs`, `turborepo-trace/xfile.rs` |
| `TURBO_TRACE_DEBUG_BIND_STEPS` | `turborepo-trace-lang-js/xfile/bind.rs` |
| `TURBO_TRACE_DEBUG_BIND_TRACE` | `turborepo-trace-lang-js/xfile/bind.rs` |
| `TURBO_TRACE_DEBUG_BIND_WIDE` | `turborepo-trace/xfile.rs` |
| `TURBO_TRACE_DEBUG_CHECK_WALK` | `turborepo-trace-store/walk.rs` |
| `TURBO_TRACE_DEBUG_CONFIG_FIELDS` | `turborepo-trace/configdiff.rs` |
| `TURBO_TRACE_DEBUG_CONFIG_IMPORTS` | `turborepo-trace/configdiff.rs`, `turborepo-trace/nextconfigdiff.rs` |
| `TURBO_TRACE_DEBUG_CONFIG_NAMES` | `turborepo-trace/confignames.rs` |
| `TURBO_TRACE_DEBUG_CWD` | `turborepo-trace/graph.rs` |
| `TURBO_TRACE_DEBUG_CWD_EXPLAINED` | `turborepo-trace-store/compose/cwd.rs` |
| `TURBO_TRACE_DEBUG_CYCLE_STATS` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_DEBUG_ASSEMBLE` | `turborepo-trace-store/build.rs` |
| `TURBO_TRACE_DEBUG_DEBUG_REUSE` | `turborepo-trace-store/filereuse.rs` |
| `TURBO_TRACE_DEBUG_DEBUG_WALK` | `turborepo-trace-store/build.rs`, `turborepo-trace-store/walk.rs` |
| `TURBO_TRACE_DEBUG_DERIVED_CACHE` | `turborepo-trace-store/compose/derived.rs`, `turborepo-trace/xfile.rs` |
| `TURBO_TRACE_DEBUG_E2E_ALL_RULES` | `turborepo-trace/e2e_static.rs` |
| `TURBO_TRACE_DEBUG_E2E_CACHE` | `turborepo-trace/e2e_cache.rs` |
| `TURBO_TRACE_DEBUG_E2E_CHECK` | `turborepo-trace/e2e_static.rs` |
| `TURBO_TRACE_DEBUG_E2E_DEBUG` | `turborepo-trace/e2e_static.rs` |
| `TURBO_TRACE_DEBUG_E2E_FOLLOW` | `turborepo-trace/e2e_static.rs` |
| `TURBO_TRACE_DEBUG_E2E_FOLLOW_ALWAYS` | `turborepo-trace/e2e.rs` |
| `TURBO_TRACE_DEBUG_E2E_REACH` | `turborepo-trace/e2e_static.rs` |
| `TURBO_TRACE_DEBUG_E2E_WHATIF` | `turborepo-trace/e2e_static.rs` |
| `TURBO_TRACE_DEBUG_ENV` | `turborepo-trace-lang-js/extract/mod.rs`, `turborepo-trace/graph.rs` |
| `TURBO_TRACE_DEBUG_ENV_REPORT` | `turborepo-trace-store/compose/env.rs` |
| `TURBO_TRACE_DEBUG_ERASE` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_EVAL_TRACE` | `turborepo-trace-frameworks/nextjs/jseval.rs` |
| `TURBO_TRACE_DEBUG_EXACT_BASE` | `turborepo-trace/lockdiff.rs` |
| `TURBO_TRACE_DEBUG_EXPORT_DIFF` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_EXPORTS` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_EXPORTS_CHECK` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_FLOW` | `turborepo-trace-lang-js/extract/mod.rs` |
| `TURBO_TRACE_DEBUG_FLOW_TIMING` | `turborepo-trace-lang-js/flow/mod.rs` |
| `TURBO_TRACE_DEBUG_FP_DUMP` | `turborepo-trace-frameworks/nextjs/build/fingerprint.rs` |
| `TURBO_TRACE_DEBUG_FULL_WALK` | `turborepo-trace-store/walk.rs` |
| `TURBO_TRACE_DEBUG_FW_LOADS` | `turborepo-trace-store/fw.rs` |
| `TURBO_TRACE_DEBUG_FW_PROJECTS` | `turborepo-trace-store/fw.rs` |
| `TURBO_TRACE_DEBUG_HINTS` | `turborepo-trace/graph.rs`, `turborepo-trace/lint/config.rs` |
| `TURBO_TRACE_DEBUG_INVOKE_DEBUG` | `turborepo-trace-frameworks/nextjs/invoke/scan.rs` |
| `TURBO_TRACE_DEBUG_INVOKE_TIMING` | `turborepo-trace-frameworks/nextjs/invoke/mod.rs`, `turborepo-trace-frameworks/nextjs/invoke/scan.rs`, `turborepo-trace-frameworks/nextjs/invoke/shared.rs` |
| `TURBO_TRACE_DEBUG_INVOKE_TRACE` | `turborepo-trace-frameworks/nextjs/invoke/scan.rs` |
| `TURBO_TRACE_DEBUG_KEY_DUMP` | `turborepo-trace-store/keydump.rs` |
| `TURBO_TRACE_DEBUG_KEY_DUMP_FILES` | `turborepo-trace-store/keydump.rs` |
| `TURBO_TRACE_DEBUG_LOADER_VIEW` | `turborepo-trace/configdiff.rs` |
| `TURBO_TRACE_DEBUG_LOADS_PACKAGE` | `turborepo-trace/loads.rs` |
| `TURBO_TRACE_DEBUG_LOADS_STEP` | `turborepo-trace/loads.rs` |
| `TURBO_TRACE_DEBUG_LOCATED_PACKAGE` | `turborepo-trace-lang-js/resolve/located.rs` |
| `TURBO_TRACE_DEBUG_MACHINE_CACHE` | `turborepo-trace/api.rs` |
| `TURBO_TRACE_DEBUG_MOCKS` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_MOCKS_TIMING` | `turborepo-trace-graph/barrel/mocks.rs` |
| `TURBO_TRACE_DEBUG_MODE` | `turborepo-trace/selection.rs` |
| `TURBO_TRACE_DEBUG_MOUNT_READS` | `turborepo-trace/affected.rs` |
| `TURBO_TRACE_DEBUG_NAMES` | `turborepo-trace/names.rs` |
| `TURBO_TRACE_DEBUG_NAMES_PACKAGE` | `turborepo-trace/affected.rs` |
| `TURBO_TRACE_DEBUG_NAMES_TASK_RUNNER` | `turborepo-trace/affected.rs` |
| `TURBO_TRACE_DEBUG_NAMES_TIMING` | `turborepo-trace-graph/names.rs`, `turborepo-trace/names.rs` |
| `TURBO_TRACE_DEBUG_NO_COMPOSE_REUSE` | `turborepo-trace-store/compose/reuse.rs` |
| `TURBO_TRACE_DEBUG_NO_COMPOSED_CACHE` | `turborepo-trace-store/compose.rs`, `turborepo-trace-store/compose/reuse.rs` |
| `TURBO_TRACE_DEBUG_NO_FILE_REUSE` | `turborepo-trace-store/filereuse.rs` |
| `TURBO_TRACE_DEBUG_NO_TREE_INDEX` | `turborepo-trace-store/build.rs` |
| `TURBO_TRACE_DEBUG_PURE_CALLS` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_PY_DECLARED` | `turborepo-trace/affected.rs` |
| `TURBO_TRACE_DEBUG_SYMBOLS` | `turborepo-trace/barrel.rs` |
| `TURBO_TRACE_DEBUG_TEMP_READS` | `turborepo-trace/affected.rs` |
| `TURBO_TRACE_DEBUG_TIMING` | `turborepo-trace-core/timing.rs` |
| `TURBO_TRACE_DEBUG_TSCONFIG_FIELDS` | `turborepo-trace/tsconfigdiff.rs` |
| `TURBO_TRACE_DEBUG_TSCONFIG_GOVERNED` | `turborepo-trace/tsgovern.rs` |
| `TURBO_TRACE_DEBUG_TSCONFIG_PROBED` | `turborepo-trace-lang-js/resolve/mod.rs` |
| `TURBO_TRACE_DEBUG_TYPE_EDGES` | `turborepo-trace/barrel.rs`, `turborepo-trace/uncertainty.rs` |
| `TURBO_TRACE_DEBUG_UNANSWERED_DIRECTORY` | `turborepo-trace-lang-js/resolve/mod.rs` |
| `TURBO_TRACE_DEBUG_UNPACK_MMAP` | `turborepo-trace-store/artifact.rs` |

Read by unit tests only:

- `TURBO_TRACE_DEBUG_SKIM_DIR` (`turborepo-trace-lang-js/lexer/tests.rs`)
- `TURBO_TRACE_DEBUG_SKIM_MUTATIONS` (`turborepo-trace-lang-js/lexer/tests.rs`)
- `TURBO_TRACE_DEBUG_TESTKIT_ITERS` (`turborepo-trace-testkit/checks.rs`)
- `TURBO_TRACE_DEBUG_TESTKIT_SEED` (`turborepo-trace-testkit/checks.rs`)

Some integration tests read further `TURBO_TRACE_DEBUG_*` variables themselves (case counts,
seeds, directories of external inputs); those are part of the tests, not of the crates.
