# Native file tasks prototype

This checkout extends Turbo itself. No Bun patches, plugins, generated
package scripts, or external scheduler are involved. `experimentalFiles` expands
a file glob into ordinary Turbo tasks; Turbo owns their scheduling, hashes,
logs, exit status, and local/remote cache entries.

Build the modified CLI with `cargo build -p turbo`. Copy this example outside the
Turborepo source checkout before running it, then initialize it:

```sh
git init
bun install
```

Use the **modified binary**, with `--skip-infer` so it cannot delegate to a stock
installation. Substitute its absolute path for `/path/to/turbo`:

```sh
# One file: its successful result seeds the ordinary Turbo cache.
/path/to/turbo run 'test@packages%2Fapp-a%2Fsrc%2Falpha.test.ts' --skip-infer

# All discovered files: alpha is reused and beta executes.
/path/to/turbo run test --skip-infer

# Inspect the generated tasks, hashes, input files, and cache state.
/path/to/turbo run test --dry=json --skip-infer
```

The tests append to `.executions` only when they actually execute. Replayed logs
do not append to it. Change `packages/shared/src/alpha.ts` and only alpha's test
is invalidated; beta remains cached. Adding a new matching file adds a new task
without invalidating existing tasks. Deleting a file removes its task.

The `test` aggregate and a directly selected child use the same child identity,
command, and inputs. Paths in child task names are percent-encoded, so selection
does not depend on discovery order or shard assignment. Standard remote caching
uses the same keys and transport as other Turbo tasks.

## Configuration

`experimentalFiles.include` contains positive globs relative to the config's
directory. Optional `experimentalFiles.exclude` removes entries before expansion;
for example, a unit run can exclude `**/*.dom.test.tsx` and
`**/*.integration.test.ts`. Root configurations use explicit `//#name` task names. Package-local
configurations can use ordinary task names; root templates are not inherited
and re-expanded in every package in this prototype. The configuration must live
in its executing package directory. A custom root configuration filename works
only when that file remains in the repository root.

Discovery registers the matching entries. Import tracing happens only when the
task graph reaches a file, including through ordinary `dependsOn` edges. A focused
run traces that file and its dependencies; a full run traces every selected file.
Shared modules are parsed and resolved once per template per invocation. There is
no persistent trace cache, so new invocations recheck edits and resolution changes.
Package-local templates can further reduce discovery work.

Glob escapes can select literal parentheses or brackets in route filenames.
Task identities retain the full encoded path; log files use a digest in a
separate directory when the ordinary filename would exceed 255 bytes.

`command` is a direct argv array containing a standalone `$TURBO_FILE$` argument.
Turbo substitutes the discovered relative path as one argument, without a shell.
The command can invoke any existing CLI that supports independent file work.

`inputs` lists shared files: runner scripts, fixtures, configuration and runtime
reads that imports cannot discover. The entry file is mandatory and added by
Turbo. `env` and `global.env` work normally; values in passthrough environment
lists are not part of the key. Pin the runtime and dependency installation in CI.

For JavaScript and TypeScript, opt into native import discovery:

```json
{
  "experimentalFiles": {
    "include": ["src/**/*.test.ts"],
    "trace": true,
    "resolution": "bun",
    "traceRoots": ["testing/setup.ts"]
  },
  "command": ["bun", "test", "--preload", "./testing/setup.ts", "$TURBO_FILE$"],
  "inputs": ["fixtures/**", "$TURBO_ROOT$/.env.test"],
  "env": ["NODE_ENV"]
}
```

`resolution: "bun"` resolves extensionless imports, workspace package exports,
package imports, nearest tsconfig aliases and inherited compiler configuration.
It includes literal `import("...")` expressions, runtime imports/re-exports,
resolution candidates and lockfiles. JSX also tracks the implicit React
production and development runtimes conservatively, including aliases and
workspace exports for those runtimes. Type-only imports are erased by Bun and
are excluded. Missing resolution candidates are retained so adding a file can
invalidate an earlier result. Turbo does this itself; Bun needs no new protocol.
The resolution profile follows [Bun's module rules](https://bun.sh/docs/runtime/module-resolution).

Each source module gets a shared graph-only input node. Execution units depend
on their reachable modules, including preload roots. Source import cycles do not
become execution cycles. A changed source file invalidates its consumers; a
private test change does not invalidate unrelated consumers. Changes to different
functions in the same source file still invalidate all consumers of that file.

The default `resolution: "explicit"` retains the smaller, runtime-neutral
contract: explicit relative ESM imports, re-exports and literal dynamic imports
with extensions. JSX needs the Bun profile or complete explicit inputs.
`trace: false` supports any language or CLI using declared inputs only.

Import discovery does not discover arbitrary filesystem reads, network state,
subprocesses or code generated at runtime. Computed imports, CommonJS loading,
custom JSX sources, unsupported loaders and traces over 1,024 source files stop
with a diagnostic. `fallbackInputs`, when present, must declare the **complete**
conservative input set for an incomplete graph; it is not just the missing import.
Turbo warns when fallback is used. Ordinary `inputs` and `dependsOn` still apply.
A detected unresolved import never silently disappears from the hash. Import tracing does not certify hermetic execution. Repository audits
that inspect Git's index must stay uncached unless their full state is modeled.

## Supported execution contract

- Opt-in, finite commands whose files can execute independently, with cached logs
  and exit status. File outputs and merged coverage are not supported and output
  declarations are rejected.
- One process per miss; successful focused runs seed full runs and vice versa.
  Cold runs can cost more than a batched runner. No worker reuse is claimed.
- Root templates use `//#name`. Package templates need a distinct name without
  inherited task defaults; templates inherited from another package are rejected.
- Shared inputs, fallback inputs and global inputs cannot exclude mandatory
  dependencies. Structured inputs and `$TURBO_EXTENDS$` are not supported here.
- Configure common arguments in `command`. Aggregate passthrough arguments and
  `--only` are rejected. Direct-child arguments get distinct cache keys.
- Entry discovery happens before task execution. Generated entry files are not
  supported. Watch mode is explicitly rejected; invoke `turbo run` again to
  rediscover additions and deletions.
- Use the Bun profile with normal module resolution. Custom resolver flags need
  `trace: false` and explicit inputs. Runtime loader overrides must also have a
  complete declared input set.
- Installed dependencies follow Turbo's immutable lockfile/install contract.
  Linked workspace sources are traced; mutable external files are unsupported.
- This is an experimental upstream implementation. Local validation is not a
  claim of upstream acceptance, hosted CI deployment or cross-platform release.

## Validation

```sh
cargo test -p turbo --test file_tasks_test --test task_command_test -- --test-threads=1
cargo test -p turborepo-turbo-json --lib
cargo clippy -p turborepo-turbo-json --lib -p turbo --test file_tasks_test -- -D warnings
```

The file-task suite records execution effects to prove focused/full reuse, selective invalidation,
new/deleted file discovery, shared-input invalidation, failure propagation, and
rejection of unsupported dynamic imports. It also proves that an unrelated file
with an unsupported import does not block a focused run, while selecting the full
family still reports that error before executing any commands.

## Cross-checkout HTTP cache proof

From the Turborepo source checkout:

```sh
python3 examples/native-file-tasks/verify.py --binary target/debug/turbo
```

This generates a temporary monorepo containing one shared package and 199
consumers, uses unmodified Bun 1.4.3, and runs Turbo against a loopback HTTP
artifact server. A separate checkout has its own local cache. The script asserts:

- A focused agent run executes one file and uploads its result.
- The separate checkout's complete run executes the other 199 files, including
  the shared package's own test, and downloads the seeded result.
- A focused agent run can reuse a result uploaded by that complete run.
- Editing the shared package's private test executes only that test.
- Editing the source module used by one consumer executes only that consumer.

The server is a test double for artifact storage; the client, hashing, upload,
download, and cache restoration are Turbo's existing implementations. This is
transport proof on one machine, not a hosted-provider or actual CI execution.

Build prerequisites are listed in the repository's `CONTRIBUTING.md`. Use the
pinned Rust toolchain and the Zig version required by the checkout.
