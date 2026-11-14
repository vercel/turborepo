# turborepo-trace-tasks

## Adding a tool to the task recognizers

Commands in package scripts, Makefiles and CI steps are matched against the tool registry (this crate). An unknown command is already sound: the task falls back to its whole package directory. A recognizer only buys precision. Pick the cheapest tier that fits:

1. **A declarative spec**: a TOML file in `crates/turborepo-trace-tasks/specs/`, interpreted by `crates/turborepo-trace-tasks/src/specs/`. Use this when the tool's inputs and outputs follow from its flags, config fields and defaults. The format reference is the module doc of `crates/turborepo-trace-tasks/src/specs/mod.rs`.
2. **A content scanner** (`crates/turborepo-trace-tasks/src/scanners/`) for file formats that need real parsing (Prisma blocks, `$ref` closures). Specs name scanners with `scan = "<name>"`.
3. **A Rust recognizer** (`crates/turborepo-trace-tasks/src/recognizers/`), only for real control flow: argv shapes that dispatch to other programs, or recursion over other tools' configs.

Steps for a spec:

1. Write `crates/turborepo-trace-tasks/specs/<id>.toml`. The file stem must equal `id`.
2. Add `("<id>", include_str!("../../specs/<id>.toml"))` to `SOURCES` in `crates/turborepo-trace-tasks/src/specs/mod.rs`.
3. Add fixture commands to `crates/turborepo-trace-testkit/fixtures/tasks/codegen/<family>/commands.txt` (one command per line) and a `family_test!` in `crates/turborepo-trace-tasks/tests/codegen.rs`. Then record the snapshot with `INSTA_UPDATE=always cargo test -p turborepo-trace-tasks --test codegen` and review it.
4. `cargo test -p turborepo-trace-tasks`. A spec that does not parse, or names an unknown value, group, flag or scanner, fails `specs::tests::specs_parse`.

Worked example, a tool without a config (json-schema-to-typescript):

```toml
# json-schema-to-typescript: `json2ts -i <schema|glob> -o <file|dir>`.
id = "json2ts"
programs = ["json2ts"]
toolchain = ["npm:json-schema-to-typescript"]
flags = ["--input|-i", "--output|-o", "--cwd", "--bannerComment"]  # flags taking a value

# The schemas and every file their `$ref`s reach are inputs (URLs are uncertain).
always = [{ scan = "refs", on = "--input" }, { output = "--output" }]
```

A tool with subcommands and a config (abridged from `drizzle-kit.toml`):

```toml
id = "drizzle-kit"
programs = ["drizzle-kit"]
toolchain = ["npm:drizzle-kit"]
flags = ["--config", "--schema", "--out"]
ignore = ["", "studio", "help"]           # subcommands with no file effect

[config]
label = "drizzle"                         # prefix of uncertainty details
flag = "--config"
names = ["drizzle.config.ts", "drizzle.config.js", "drizzle.config.json"]

[values]                                  # sources in precedence: flag, field, pkg, default
schema = { flag = "--schema", field = "schema", base = "cwd", required = true }
out = { flag = "--out", field = "out", base = "cwd", default = "drizzle" }

[groups]
schema = [{ input = "schema", code = true, glob = "**/*.ts", reason = "drizzle schema" }]

[sub]
generate = ["schema", { membership = "out", join = "meta", reason = "drizzle snapshots" }, { output = "out" }]
push = ["schema"]
```

Anything that is not statically known is handled for you: a non-static config value, a missing required field, an unreadable config, a URL, or an unknown subcommand. It becomes an uncertainty plus a conservative membership. A generator that writes an installed package (Prisma's client into `node_modules`) declares it with `TaskSpec::output_package` (in a scanner or Rust recognizer). The graph store then links the package's opaque node, its `node_modules` locations and its bare imports to the task.

**What a command does with its inputs** decides whether their uncertainty reaches the consumers of the task's outputs. Three cases:

| The command… | Record it as | Edge in the graph |
|---|---|---|
| runs the file (a script, a code config, a test file a runner discovers) | an entry (`TaskSpec::entry`), or a plain membership | plain: changes and uncertainty |
| compiles, bundles or copies it into an output that is run later (`tsc` emitting JavaScript, a bundler) | an entry or a plain membership or input | plain: the importer of the output runs a copy |
| only reads it (a type-check, a linter, a hash input such as turbo `inputs`) | `Membership::data = true`, `TaskSpec::data_inputs` | data edge (flag `data`): changes only |

Plain is the default, and the whole-package fallback of an unknown command is plain: it may do any of the three. Mark a membership or input as data only when the tool cannot run the files or write code made from them. The registry does it for whole recognizers listed in `READ_ONLY` (`crates/turborepo-trace-tasks/src/registry.rs`); `tsc` decides per project from its emit options. A task made only of `tsc -b` project parts is linked as the producer of what its parts write, not of everything it declares: see `may_write_into` in `crates/turborepo-trace-tasks/src/lib.rs`. Fixture and tests: `crates/turborepo-trace-testkit/fixtures/tasks/data-inputs`, `crates/turborepo-trace-tasks/tests/data_inputs.rs`.

A recognizer for a **test runner** also records which tests the command executes, with `TaskSpec::test_run(<framework>, <dir>)`: the framework id of the graph's test records (`jest`, `vitest`, `pytest`, `gotest`, `cargo-test`) or the runner's own name, and the directory the run is rooted at (filters such as shards only narrow it). That is the "runs" relation (`crates/turborepo-trace-tasks/src/runs.rs`): when a CI job's definition changes, `affected` selects the tests the job runs. Commands that start other tasks need nothing: every task dependency a command adds is also a run reference, and `turbo run` follows `dependsOn` in the root `turbo.json`. Commands the trace can't see through (unknown programs, scripts run by an interpreter) are scanned for launch words; the list is in `runs::launch_word`. A recognizer also names the files that *define* the command with `TaskSpec::defines(<file>)`: the runner's config, the script an interpreter runs. Never what the command reads (sources, the modules a script imports): those are inputs, and a changed input must not count as a changed way of running tests. Fixture and tests: `crates/turborepo-trace-testkit/fixtures/tasks/gha-runs`, `crates/turborepo-trace-tasks/tests/runs.rs`.

