# PLAN.md — Task `command` Overrides

## Problem

Turborepo synthesizes task commands from toolchain conventions: package.json
scripts for JavaScript, verb tables for Cargo (`test` → `cargo test
--workspace`). There is no way to redefine what a task runs.

Motivating case: running Rust tests through `cargo nextest run`. Cargo has no
native remapping (aliases cannot shadow built-in commands, by explicit cargo
policy), and baking nextest detection into turbo makes the verb table a
registry of third-party tools — next comes `cargo-llvm-cov`, then miri. The
tool-agnostic fix: turbo.json — turbo's own task-definition surface — defines
the command.

## Principle

**`command` replaces the process argv. The toolchain keeps owning everything
else.**

A task command is two separable things:

1. **argv** — the process to spawn
2. **frame** — everything the toolchain wraps around it: working directory,
   serial grouping, hash wiring (crate-closure input globs,
   `HASHED_ENV_VARS`, external-dependency hashing), env composition
   (compile-cache injection, MFE vars), display

`command` swaps only the argv. An overridden `acme#test` still hashes by the
crate closure, still serializes with other cargo invocations, still receives
`RUSTC_WRAPPER` injection when the compile cache runs. The frame is untouched
— that is what makes one field behave identically across toolchains.

## Prerequisite: toolchain naming (PR 0 — shipped, #13311)

Toolchains are named by **language**, not build tool — the axis users think
in, and the one with an answer for every ecosystem (prior art: moonrepo,
Pants):

- `ToolchainId::JAVASCRIPT` stays `"javascript"`.
- `ToolchainId::CARGO` renames `"cargo"` → `"rust"`. Safe now: every
  surface is behind `experimentalCargoWorkspaces`. The flag keeps its name —
  it describes the feature (Cargo workspace support), not the toolchain
  identity.

**Alias** (resolved at parse time): `typescript` → `javascript`. Using an
alias and its canonical name in the same map is an error:

```
Error: `command` defines both "typescript" and "javascript" — these are the
same toolchain. Use one key.
```

There is no `cargo` alias: a `"cargo"` key gets the unknown-toolchain error
with a did-you-mean pointing at `"rust"` — corrected, not accepted.

## Prerequisite: the workspace package is named by the user (PR 0.5)

The synthetic Cargo workspace package previously took a magic reserved name
(`cargo`, then `rust`). Magic names are a design defect: they collide with
real packages (today a crate named `rust` is silently skipped), they confuse
(`rust#test` names a package nobody defined), and they overload the
toolchain id.

**Rule: using Turborepo with Rust requires naming the Cargo workspace**, in
the root `Cargo.toml`, via cargo's sanctioned free-form surface:

```toml
[workspace]
members = ["crates/*"]

[workspace.metadata]
name = "acme"
```

Task keys, filters, and the TUI use that name: `acme#test`,
`--filter=acme`. This is not extra ceremony — it is the rule every package
already follows (JS packages without a `name` are errors); cargo just has
no native slot for this particular package's name, so we designate one.
The key is deliberately **un-namespaced** (not `metadata.turbo`): a
workspace's name is a property of the workspace with universally shared
semantics, not tool-specific configuration. No existing Rust tool names
workspaces at all, so this mints an agnostic proto-convention; if cargo
ever grows native workspace names, we read those and deprecate this.

- **One source, no fallbacks.** A root `[package] name` is _not_ consulted:
  in non-virtual workspaces the root package is also a real crate, and
  reusing its name would conflate the two (and mint two packages with one
  name at one directory).
- **Missing name = hard, actionable error** at any turbo invocation while
  the flag is on:

  ```
  Error: The Cargo workspace has no name.

  Turborepo needs a name for the workspace's tasks (`<name>#test`),
  filters (`--filter=<name>`), and configuration. Add one to the root
  Cargo.toml:

      [workspace.metadata]
      name = "my-workspace"
  ```

- **Collisions get honest.** A workspace name that matches any crate or JS
  package is a discovery error naming both parties. The reserved-name
  skip-with-warning hack is deleted.
- **Validation**: non-empty, no `#`, not `//`, unique across the graph.
  Warn (not error) on `rust`/`javascript` — legal, but re-introduces the
  confusion this removes.
- **Disambiguation this creates**: the _toolchain id_ (`"rust"`,
  `"javascript"`) keys the per-toolchain `command` map and never appears as
  a package name; the _workspace package name_ is the user's.
- **Dogfood**: this repository names its workspace `turborepo-crates`.

Examples throughout this document use a workspace named `acme`.

## API

```jsonc
// turbo.json (root)
{
  "futureFlags": { "experimentalTaskCommand": true },
  "tasks": {
    /* see forms below */
  }
}
```

- **Shape**: argv array — executed directly, no shell, no `&&`, no
  interpolation. It is a _command_, not a _script_ (a "script" implies shell
  semantics package.json owns; we deliberately don't provide them).
- **Flag**: using `command` without `futureFlags.experimentalTaskCommand` is
  a **hard error** — never a silent strip, because ignoring it would change
  _what executes_. (The silent-strip pattern used for `incremental` is only
  safe for optimizations.) Gating hook exists:
  `ProcessedTaskDefinition::from_raw` already receives `&FutureFlags`
  (processed.rs:599).
- **Schema**: `#[schemars(skip)]` while experimental.

### The three value forms

```jsonc
// 1. Argv array — allowed everywhere.
//    At unscoped root: the default for EVERY package, all toolchains.
//    Always valid regardless of repo composition (a toolchain-agnostic
//    command like this is exactly the mixed-repo use case).
"clean": { "command": ["rm", "-rf", "dist", ".turbo"] },

// 2. Per-toolchain map — unscoped root only (scoped positions already
//    know their toolchain; a map there is noise → validation error).
"test": {
  "command": {
    "rust": ["cargo", "nextest", "run"],
    "javascript": ["vitest", "run"]
  }
},

// 3. Opt-out — null or [], scoped positions only (a default of nothing
//    is meaningless → validation error at unscoped root).
"acme#test": { "command": null }
```

Raw schema is a tri-state (serde must distinguish _absent_ from _null_):
`RawCommand::{Argv(Vec<Spanned<UnescapedString>>), OptOut, PerToolchain(Map)}`
— dispatched on JSON shape (array / null / object) via biome-deserialize.

### Execution model

**`command` is the complete argv: element 0 is the program, the rest are its
arguments. Nothing is prepended, by any toolchain.**

```jsonc
"web#thing": { "command": ["node", "--run", "thing", "stuff"] }
// spawns: node --run thing stuff        (cwd: packages/web)
// NOT:    pnpm run node --run thing stuff
```

The JS toolchain's `pnpm run <task>` construction is what the override
_replaces_ — exactly as the cargo verb table is what an `acme#test` override
replaces. Keeping the package-manager indirection would make the field mean
"script name to pass to the package manager", a different and weaker feature.

- **Program resolution**: argv[0] resolves via the OS's normal spawn
  semantics — absolute paths as-is, bare names through `PATH`, relative
  paths (`["./scripts/build.sh"]`) against the cwd, which is the package
  directory.
- **Uniformity**: this rule is toolchain-independent. The toolchain
  contributes the frame (cwd, serial group, env composition, hash wiring) —
  never argv content.
- Aside: Node 22+'s `node --run <script>` reads package.json scripts and
  adds `node_modules/.bin` to `PATH` itself — a legitimate way to run a
  script with `.bin` resolution but without the package-manager process.

## Precedence (highest → lowest)

| #   | Source                                                           | Nature                                                             |
| --- | ---------------------------------------------------------------- | ------------------------------------------------------------------ |
| 1   | `command` in a Package Configuration                             | package explicitly redefines itself                                |
| 2   | `command` on a package-scoped root key (`web#test`, `acme#test`) | root explicitly targets one package                                |
| 3   | **Package-authored** native definition — package.json script     | the toolchain's native, user-written definition wins over defaults |
| 4   | `command` on an unscoped root task (array or map)                | toolchain-agnostic / per-toolchain default                         |
| 5   | **Toolchain-synthesized** command — cargo verb table             | turbo's fallback, authored by nobody                               |

Cargo has nothing at level 3 (crates don't author task commands), so an
unscoped `rust` command beats the verb table:
`"test": { "command": { "rust": ["cargo", "nextest", "run"] } }` means
`turbo test` runs nextest; without it, the verb table gives `cargo test`.
For JS, a package.json script shadows the unscoped default — leaning into
what the toolchain does natively.

Resolution consults levels 3 and 5 through the toolchain (`defines_task` /
verb tables), so all five levels meet in **one resolver function** — the
single point of truth, heavily tested.

## Semantics matrix

### A. Explicit override vs native (levels 1–2 vs 3)

```jsonc
// JS package `web`, package.json: "scripts": { "test": "jest" }
"web#test": { "command": ["vitest", "run"] }
// Runs: vitest run (cwd packages/web). jest never enters the picture. Silent.

// Cargo workspace package:
"acme#test": { "command": ["cargo", "nextest", "run", "--workspace"] }
// Runs nextest, serialized with other cargo tasks, crate-closure hashed.
```

### B. Unscoped defaults (level 4) and the "every package" rule

An unscoped `command` grants the task to every package it covers (bare
array: all packages; map: packages of the listed toolchains). Two canonical
Rust-test patterns:

```jsonc
// Pattern A — one workspace-wide run (current CI shape):
"acme#test": { "command": ["cargo", "nextest", "run", "--workspace"] }

// Pattern B — per-crate test tasks, individually cached and
// affected-aware; opt the workspace package out to avoid running the
// suite twice:
"test": { "command": { "rust": ["cargo", "nextest", "run"] } },
"acme#test": { "command": null }
// Each crate runs `cargo nextest run` from its own directory (cwd =
// package dir ⇒ that crate's tests), serialized on the cargo group.
```

### C. Tasks that exist only through `command`

```jsonc
// No "coverage" verb exists; now the task does:
"acme#coverage": {
  "command": ["cargo", "llvm-cov", "--workspace"],
  "outputs": ["coverage/**"]
},
// Library crates map no verbs; command gives them one:
"turborepo-scm#fuzz": { "command": ["cargo", "fuzz", "run", "scm"] }
```

Existence surfaces that compose
`command.is_some() || toolchain.defines_task(...)`: engine builder
(`defines_task`, definitions.rs:217 — global-deps hashing), TUI task list
(`tasks_with_command`), watch mode (same call).

### D. Package Configurations

```jsonc
// packages/web/turbo.json
{
  "extends": ["//"],
  "tasks": {
    "test": { "command": ["vitest", "run", "--shard=1/2"] } // level 1
  }
}
```

Unscoped is the natural form here (the file scopes it); `pkg#task` syntax
stays forbidden in Package Configurations (existing
`UnnecessaryPackageTaskSyntax`). Map form: forbidden (toolchain already
known). The synthetic `rust` package's "package config" _is_ the root
turbo.json (root-at-repo-root reuse, loader.rs:471-478) — `acme#test` in
root is its only configuration point.

### E. Merge across the extends chain

`command` merges **atomically** (scalar semantics, extend.rs `set_field!`):
the most specific definition's whole value wins; maps never deep-merge.
`$TURBO_EXTENDS$` inside `command` is a validation error (appending argv
fragments has no coherent meaning). Within one file, key specificity is
unchanged: `web#test` beats `test` (either/or lookup, lib.rs:238-255).

Other fields remain independently mergeable — a package can override
`command` while inheriting root's `outputs`.

### F. The frame, per toolchain

| Frame property                  | JavaScript                                                                                       | Rust                                                                                                                    |
| ------------------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------- |
| cwd                             | package directory                                                                                | package directory (the workspace package's dir _is_ the repo root)                                                      |
| serial group                    | none                                                                                             | `"cargo"` iff `argv[0] == "cargo"` — the toolchain knowing its own binary's build-dir locking, not third-party sniffing |
| env                             | per env-mode, as today                                                                           | per env-mode, as today                                                                                                  |
| compile-cache injection         | n/a                                                                                              | applies (conflict classification already tolerates/detects user wrappers)                                               |
| PATH                            | **verbatim — no `node_modules/.bin`** (override bypasses the package manager; documented v1 gap) | verbatim                                                                                                                |
| hash wiring (`derived_task_io`) | unchanged                                                                                        | crate-closure globs, `HASHED_ENV_VARS`, external-dep hashing — unchanged                                                |

### G. Pass-through args

Appended verbatim — turbo can't know an arbitrary command's separator
convention:

```
turbo test -- -E 'test(scm)'
# → cargo nextest run --workspace -E 'test(scm)'
```

With a per-toolchain map, pass-through args append to **every** toolchain's
command in the run — `turbo test -- --watch` reaches both vitest and
nextest. Identical to today's behavior with heterogeneous package scripts,
not new. Hashed as today (`pass_through_args` is already in `TaskHashable`).

### H. Data from the outside

**1. Environment variables reaching the process — works, per env-mode
(frame, unchanged).** Everything in loose mode; declared `env` /
`passThroughEnv` / `globalPassThroughEnv` in strict mode. The existing
hashing rule applies unchanged — if the variable changes behavior, declare
it:

```jsonc
"test": {
  "command": { "rust": ["cargo", "nextest", "run"] },
  "env": ["NEXTEST_PROFILE"]   // hashed: profile change ⇒ cache miss
}
```

```bash
NEXTEST_PROFILE=ci turbo test   # nextest reads it from its environment
```

**2. Ad-hoc CLI flags — works, via pass-through args** (section G).

**3. Interpolating variables _into_ the argv — deliberately no.**

```jsonc
"test": { "command": ["cargo", "nextest", "run", "--profile", "$PROFILE"] }
// passes the literal string "$PROFILE" — there is no shell
```

No-shell is the core contract (cross-platform determinism, no quoting hell,
hashable-as-written). Interpolation would force answers to: Windows `%VAR%`
vs `$VAR`, escaping literal dollars, and — worst — whether the _hash_
covers the template or the resolved value (resolved ⇒ per-machine cache
divergence unless the var is also declared; template ⇒ stale caches when
the var changes). That's a shell's job. Two explicit escape hatches:

```jsonc
// (a) Most tools read configuration from env directly — case 1 covers it:
//     NEXTEST_PROFILE=ci turbo test

// (b) If you truly need shell semantics, say so — you own the shell:
"deploy": { "command": ["bash", "-c", "deploy.sh --target $DEPLOY_TARGET"] }
```

Friendliness: a **warning** (not error) when an argv element matches
`$IDENTIFIER` / `%IDENTIFIER%` — _"command arguments are not
shell-interpolated; `$PROFILE` will be passed literally"_ — since a literal
dollar-argument is almost always a mistake, but occasionally legitimate.

### I. Hashing

The resolved command joins `TaskHashable` (new field + capnp schema;
`TaskDefinitionHashInfo` gains a `command()` accessor, turborepo-types
lib.rs:1269-1303). Editing a command invalidates exactly the affected tasks.
Capnp change ⇒ one-time global hash bust on upgrade (standard).

### J. Presentation

Run summary / dry-run show the joined resolved argv. TUI shows the task via
existence composition. `--graph` unchanged (graph shape is
command-independent except for tasks existing only via `command`).

### K. Single-package mode

Unscoped is the only syntax and unambiguous — array and opt-out allowed,
map allowed (degenerate but harmless).

## Validation matrix (config-load time)

| Condition                                                       | Outcome                                                              |
| --------------------------------------------------------------- | -------------------------------------------------------------------- |
| `command` present, flag off                                     | error: requires `futureFlags.experimentalTaskCommand`                |
| Map form in scoped position (root `pkg#task` or Package Config) | error: toolchain already determined; use an array                    |
| `null` / `[]` at unscoped root                                  | error: a default of nothing is meaningless                           |
| Unknown toolchain key                                           | error + did-you-mean (`"cargo"` → `"rust"`; known toolchains listed) |
| Alias + canonical in one map (`typescript` + `javascript`)      | error: same toolchain, use one key                                   |
| `rust` key without `experimentalCargoWorkspaces`                | error pointing at the flag                                           |
| `$TURBO_EXTENDS$` inside `command`                              | error: command is atomic                                             |
| Empty string element in argv                                    | error                                                                |
| Argv element looks like `$VAR` / `%VAR%`                        | warning: passed literally, not interpolated                          |

## Implementation map

| Layer                                         | Change                                                                                                                                                                                                                                             |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `turborepo-repository` (PR 0, shipped #13311) | `ToolchainId` `"cargo"`→`"rust"`                                                                                                                                                                                                                   |
| `turborepo-repository` (PR 0.5)               | delete `WORKSPACE_PACKAGE_NAME`; read `[workspace.metadata] name` during discovery (`cargo metadata` emits `workspace_metadata`); missing-name error; collision + validation checks; dogfood `name = "turborepo-crates"` in this repo's Cargo.toml |
| `turborepo-turbo-json`                        | `RawTaskDefinition.command` tri-state; alias resolution; atomic merge; flag gate + validation matrix in `from_raw` (receives `&FutureFlags` already)                                                                                               |
| `turborepo-types`                             | `TaskDefinition.command`; `TaskDefinitionHashInfo::command()`                                                                                                                                                                                      |
| `turborepo-engine`                            | `from_processed`; five-level resolver; `defines_task` composition (definitions.rs:217)                                                                                                                                                             |
| `turborepo-repository`                        | `Toolchain::task_command`/`task_display_command` gain `override_command: Option<&[String]>`; JS + Cargo place it in their frames; shared argv-split helper                                                                                         |
| `turborepo-task-executor`                     | `ToolchainCommandProvider` receives resolved-override lookup (`HashMap<TaskId, Vec<String>>` built by the visitor — no engine dependency)                                                                                                          |
| `turborepo-lib`                               | visitor builds the lookup (exec.rs:56-62); TUI `tasks_with_command` composition                                                                                                                                                                    |
| `turborepo-task-hash`                         | `TaskHashable.command` + proto.capnp + population (lib.rs:538-552)                                                                                                                                                                                 |

## Test plan

- naming (PR 0.5): missing `[workspace.metadata] name` ⇒ actionable error;
  name collision with a crate / JS package ⇒ discovery error; invalid names
  (`#`, `//`, empty) rejected; `rust`/`javascript` warn; fixtures renamed
- turbo-json: tri-state parse (array/null/map), alias resolution + conflict,
  atomic merge, full validation matrix, Package Config scoping
- resolver: all five precedence levels, per-toolchain map fan-out, opt-out,
  script-shadows-default, verb-table-loses-to-default
- toolchain unit: JS override (cwd, bypasses pm), cargo override
  (serial-group heuristic both ways), library-crate command
- engine/TUI: existence composition
- hashing: ±command ⇒ hash change; map key change ⇒ only that toolchain's
  tasks invalidate
- e2e: `acme#test` nextest override + Pattern B fixture; dry-run JSON shows
  argv
- dogfood: this repo adopts Pattern A (then evaluates B) after a canary
  knows the field

## Rollout

1. `unknown_fields = deny` ⇒ turbos predating the field hard-error on
   turbo.jsons using it. Sequence: land → canary → adoption (same dance as
   every field/flag).
2. PR series: **PR 0** rename (shipped) → **PR 0.5** workspace naming →
   **PR 1** schema/flag/validation → **PR 2**
   resolver + toolchain/executor plumbing + hashing → **PR 3** dogfood flip.

## Out of scope (recorded)

- Shell-string form (a future `script` field would be the honest spelling)
- `node_modules/.bin` PATH augmentation for JS overrides
- Per-toolchain pass-through separator awareness
- Batching contended cargo invocations (parked separately)
