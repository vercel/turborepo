# Experiment: Can Turborepo auto-detect env vars by instrumenting spawned processes?

**Question:** Users must hand-maintain `env: [...]` lists in `turbo.json`. Could
Turborepo instead observe which environment variables a task's process tree
actually reads, and derive the list automatically?

**Verdict:** Instrumentation is **workable as a suggestion/lint engine, and
provably unworkable as an automatic source of truth for the task hash.** Two
independent walls stand in the way of full automation:

1. **Coverage:** there is no universal observation point. Env access is a plain
   memory read with no syscall, so the kernel can't see it, and each runtime
   reads the environment differently — several popular ones (shells, Python,
   Bun, Go, any static binary — including Turborepo's own shipped binary)
   bypass every hook available to an external observer.
2. **Soundness:** even with perfect observation, the data arrives *after* the
   task runs, while the hash is needed *before*; and the observed set is a
   function of which code path executed, which is itself a function of the
   environment. A recorded list is therefore valid only for environments
   similar to the one it was recorded in — exactly the wrong property for a
   cache key, where the dangerous case is the *other* environment.

Everything below was verified empirically in this repo's CI container
(Linux x86-64, glibc, node 22.22, npm 10.9, bun, python 3, go 1.24, rustc,
turbo 2.10.8). Reproduce the core matrix with `./run-matrix.sh`.

---

## Experiment 1 — Is env access visible at the syscall layer? (No.)

`strace -f` on a C program that calls `getenv("MY_SECRET_TOKEN")` and prints it:

```
syscalls total: 37
lines mentioning MY_SECRET_TOKEN: 1
8873  write(1, "MY_SECRET_TOKEN=hunter2\n", 24) = 24
```

The only syscall mentioning the variable is the program's own `printf`. The
environment is copied into the child's address space at `execve(2)` and read
as ordinary memory from then on. **Conclusion: strace, seccomp, and eBPF
syscall tracepoints are categorically blind to env reads.** The only
kernel-side option would be uprobes on libc's `getenv` symbol, which requires
root and has exactly the same blind spots as Experiment 2's approach.

## Experiment 2 — LD_PRELOAD interposition on `getenv` (the runtime matrix)

We built `shim.c`, an `LD_PRELOAD` library interposing `getenv`/`secure_getenv`
and logging `pid \t progname \t varname` per call, then ran nine runtimes that
each read `MY_SECRET_TOKEN`:

| Runtime | Read observed? | Why |
|---|---|---|
| C, dynamically linked | ✅ CAUGHT | calls `getenv@plt` |
| Rust (`std::env::var`) | ✅ CAUGHT | Rust std calls libc `getenv` |
| **Node 22** (`process.env.X`) | ✅ CAUGHT | every JS read → `uv_os_getenv` → `getenv` (verified 3 reads → 3 calls: live, not cached) |
| C, statically linked | ❌ MISSED | `getenv` linked into the binary; no PLT to interpose |
| Go (CGO_ENABLED=0) | ❌ MISSED | Go runtime snapshots `environ` at startup, never calls libc |
| **Bun** | ❌ MISSED | 593 getenv calls logged — all JSC/WebKit internals; `process.env` reads use Zig's own environ scan |
| **Python 3** | ❌ MISSED | `os.environ` is a dict snapshotted at interpreter startup; the 40 logged calls are CPython startup housekeeping |
| **bash** | ❌ MISSED | bash *defines its own `getenv`* (visible in its dynsym) and serves `$FOO` from its internal variable table |
| dash (`sh`) | ❌ MISSED | same: environ imported into shell variables at startup, zero libc getenv calls |
| turbo's shipped binary | ❌ (0 calls) | statically linked musl — the tool itself is un-instrumentable this way |

Two implementation traps we hit that anyone building this would hit:

- **`secure_getenv`-first initialization order.** Node initializes OpenSSL
  before anything else; OpenSSL uses `secure_getenv`. Our first shim resolved
  its config on first call and cached NULL forever, silently recording nothing
  for Node. Interposers must handle multiple entry symbols and lazy init.
- **Node ≥20 caches nothing** (good for this technique), but worker threads
  get a snapshot copy of `process.env`, so reads inside `worker_threads` are
  another gap at the JS layer.

**Conclusion: libc interposition sees Node and Rust perfectly and misses
shells, Python, Bun, Go, and static binaries entirely.** Every npm script is
`sh -c "..."` at its root, so the very first process of every task is already
in the blind zone. macOS narrows this further (`DYLD_INSERT_LIBRARIES` is
stripped by SIP for protected binaries) and Windows has no equivalent
mechanism at all (would need Detours-style IAT patching per process).

## Experiment 3 — Node-level instrumentation: `process.env` Proxy via `NODE_OPTIONS=--require`

Since most Turborepo tasks bottom out in Node, we tested wrapping
`process.env` in a Proxy that records `get`/`has`/`ownKeys` (see
`audit-node.cjs`). Findings:

- ✅ Catches property reads, `'X' in process.env` checks, and destructuring.
- ✅ Propagates automatically to child Node processes (`NODE_OPTIONS` is
  inherited), and records lookups **even when the variable is unset** — so it
  can run under strict env mode and report "the task looked for `API_URL` and
  didn't find it," which is ideal for suggestions.
- ❌ **It silently killed npm.** `npm --version` exited 1 with zero bytes of
  output under the shim. Root cause (bisected with a trap-tracing Proxy, died
  at op 372 `set HOME`): assignment through a Proxy routes through the
  `defineProperty` trap with the proxy as receiver, and Node's exotic env
  object rejects it — `'process.env' only accepts a configurable, writable,
  and enumerable data descriptor`. npm sets dozens of `npm_config_*` vars, hit
  this immediately, and its exit handler swallowed the error. Fixable (write
  directly to the target in the `set` trap), but it demonstrates how invasive
  the approach is: an unpatched edge case doesn't degrade gracefully, it
  breaks user builds in undebuggable ways.
- ❌ **The env-copy false-positive explosion.** Anything that spawns a child
  with `env: { ...process.env, EXTRA: 1 }` — which npm itself, and virtually
  every task runner and test framework, does — enumerates every key. In our
  container that single spread marked all ~130 session vars, including
  `AWS_SECRET_ACCESS_KEY` and `GITHUB_TOKEN`, as "accessed."

## Experiment 4 — End-to-end: instrumented `turbo run build`

Fixture: two workspaces. `web`'s build is a Node script that reads `API_URL`
always and `FLAG_SECRET` only when `ENABLE_FLAG` is set; `docs`'s build is a
pure shell script reading `$DOCS_TOKEN`.

- **Strict mode (default) blocks outside injection entirely:** turbo stripped
  `LD_PRELOAD`, `NODE_OPTIONS`, and our log-path vars from task envs; zero
  observations from any task process. Not a blocker for a built-in feature —
  turbo constructs the child env and could inject its own instrumentation —
  but a third-party tool cannot do this from the outside.
- **Loose mode, both layers active, per-pid analysis of the JS log:**

  ```
  pid 17343 (turbo bin wrapper): 1 ownKeys enumeration, touched all 4 fixture vars  ← noise
  pid 17364 (npm):               2 enumerations,        touched all 4              ← noise
  pid 17365 (npm):               2 enumerations,        touched all 4              ← noise
  pid 17388 (build.js):          0 enumerations, touched API_URL, ENABLE_FLAG, FLAG_SECRET  ← exact true set
  ```

  Filtering out enumerating processes recovers `web`'s true dependency set
  **exactly**. That heuristic is the strongest positive result of this
  investigation.
- **`DOCS_TOKEN` never appeared in any log.** The shell task's real
  dependency is invisible at every layer (see Experiment 2, shells).
- **Overhead was below measurement noise** on this fixture (three runs each:
  370–394 ms uninstrumented vs 326–329 ms instrumented; i.e., indistinguishable).

## Experiment 5 — Soundness: the recorded set depends on the environment itself

`conditional.js` reads `CI_DEPLOY_KEY` only when `CI` is set. Recorded
dependency sets (hermetic env, Node proxy recorder):

```
recorded on dev machine (CI unset): { CI }
recorded on CI machine  (CI=1):     { CI, CI_DEPLOY_KEY, FORCE_COLOR }
```

A list learned on one machine and used as a hash input on another silently
omits the variables that only matter on the other machine. Concretely: learn
on dev → hash ignores `CI_DEPLOY_KEY` → two CI runs with different deploy keys
produce identical hashes → **wrong cache hit, stale artifact shipped**. This
is not an implementation bug; it's inherent to observing one execution path of
a program whose paths are selected by the very inputs being inferred. (The
`FORCE_COLOR` read comes from Node internals — even hermetic recordings
include runtime housekeeping that a human would never declare.)

There is also a bootstrap problem in the same category: the hash must be
computed before the task runs, but observations exist only after it has run at
least once, and only for the code paths that run happened to take.

---

## Approaches compared

| Approach | Sees | Blind to | Portability | Risk |
|---|---|---|---|---|
| strace / seccomp / eBPF syscall tracing | nothing (env reads make no syscalls) | everything | — | none; just useless |
| eBPF uprobe on libc `getenv` | same as LD_PRELOAD | same as LD_PRELOAD | Linux only, needs root/CAP_BPF | unusable on dev machines & most CI |
| `LD_PRELOAD` getenv shim | Node, Rust, dynamic C | shells, Python, Bun, Go, static bins | Linux/macOS-minus-SIP; nothing on Windows | low overhead; silent gaps |
| `NODE_OPTIONS --require` env Proxy | all JS reads incl. missing-var lookups; auto-propagates to Node children | every non-Node process; worker_threads | all OSes (Node-only) | broke npm silently until Proxy set-trap fixed; spread/enumeration false positives |
| Static analysis (existing `eslint-plugin-turbo` `no-undeclared-env-vars`) | literal `process.env.X` in source | dynamic keys, non-JS, node_modules code | universal | none at runtime; incomplete |
| Learn-mode hashing (record run N, hash run N+1) | — | everything the recorded path didn't touch | — | **unsound cache keys** (Experiment 5) |

## What this means for the product idea

**Don't** feed observations into the hash automatically. Experiment 5 shows
the failure mode is a silent stale-cache hit — the exact class of bug the
`env` key exists to prevent, now unfixable by the user because no
configuration is visibly wrong.

**Do** consider instrumentation as a *detection and suggestion* layer, where
every limitation becomes acceptable because a human confirms the result:

- `turbo run build --learn-env` (or an `env: "infer-suggest"` mode): run the
  task in loose mode with the Node Proxy injected by turbo itself (strict-mode
  stripping doesn't apply to the spawner), filter out enumerating processes
  (the heuristic that recovered the exact true set in Experiment 4), diff
  against declared `env`, and print/`--fix` a suggested list.
- Run it under **strict** mode with turbo-injected instrumentation to report
  "task looked up `API_URL` but it was filtered" — this catches missing
  declarations at the moment they bite, with zero false negatives for Node
  tasks, and is arguably the best UX of everything tested.
- Keep static analysis (`eslint-plugin-turbo`) as the complementary signal:
  it sees code paths that didn't execute — precisely the gap runtime
  observation can't close — while runtime observation sees dynamic keys and
  dependencies' code, which static analysis can't.
- Coverage caveat to document honestly: suggestions cover Node processes
  only. Shell/Python/Go/Bun steps need the other signals or manual entry.

## Part 2 — Caching specifically: is there any sound design at all?

The analysis above rules out one design: *learn a static `env` list from a
recorded run and feed it into the hash*. Experiment 5 kills that forever.
But for caching there is a second design the first analysis glossed over,
borrowed from build systems that already cache tasks with dynamically
discovered dependencies (`gcc -MD` depfiles, Buck2 dep-files, Shake's
constructive traces):

**Value-keyed traces.** Don't learn a list — record, per cache entry, the
pairs `(var, value-observed-at-record-time)`. Lookup becomes: compute the
static hash (files, deps, script text) → fetch candidate entries → an entry
hits only if every recorded var has the same value in the environment turbo
is about to hand the task.

Two properties make this attractive for turbo, and both dissolve objections
from Part 1:

- **The bootstrap/timing problem disappears.** Turbo constructs the child
  environment before spawning, so validating a trace is a dictionary lookup
  against values turbo already holds — no need to observe anything before
  the run. The first run is a miss anyway; that's when the trace is recorded.
- **The path-dependence counterexample dissolves.** Record on dev:
  `{CI: unset}` → artifact A. On CI, `CI=1` mismatches → miss → run, record
  `{CI: "1", CI_DEPLOY_KEY: "xyz"}` → entry B. A later run with a different
  deploy key mismatches both entries → correct rebuild. For a deterministic
  task, if every recorded read sees an equal value, execution takes the same
  path, reads the same set, and produces the same output. That's the standard
  depfile soundness argument, and conditional access is handled *by
  construction*.

The soundness argument has exactly one load-bearing premise: **the trace must
contain every env read the executed path made.** Part 1 proves that premise
fails in general (shells, Python, Bun, Go, static binaries are unobservable).
So the design question becomes: *can turbo detect when the premise fails and
bail to a conservative miss instead of returning a wrong hit?*

### Experiment 6 — Escape detection: censusing the process tree

`execsnoop.c` is an LD_PRELOAD shim in which every dynamically linked process
self-announces its executable path at load, and interposed
`execve`/`execvp`/`posix_spawn(p)` calls log the binary about to be run. A
full `turbo run build` under it produced a complete census:

```
21284  self  /opt/node22/bin/node      ← turbo's bin wrapper
21291  exec  .../@turbo/linux-64/bin/turbo   ← static binary spawn VISIBLE from parent
21302  self  /usr/bin/git
21304  self  /opt/node22/bin/node      ← npm (web)
21305  self  /opt/node22/bin/node      ← npm (docs)
21326  self  /usr/bin/dash             ← task shell (web)
21327  self  /usr/bin/dash             ← task shell (docs)
21328  self  /opt/node22/bin/node      ← build.js
```

Every process is either self-announced (instrumentation loaded) or visible as
an exec target from an instrumented parent — including the spawn of the
statically-linked turbo binary, precisely the kind of process whose *reads*
are invisible. In the real feature turbo is the orchestrator and knows its
direct children natively, so the root of the tree is covered by construction.
**Escapes from observability are detectable at spawn time; a sound
conservative bailout is implementable.**

### The resulting design space for cache-correct automation

A task tree is *verifiable* when every env-reading process in it is one turbo
can fully observe. Given the census, classify each process:

1. **Node processes** — fully observable via the (npm-safe) Proxy recorder,
   including reads of vars that turn out to be unset, and `set` operations
   (needed to exclude self-set keys — e.g. `dotenv` loads `.env` into
   `process.env` before code reads it; those reads validate against the
   `.env` *file* (already hashable via `inputs`), not the parent env).
2. **The npm-injected `sh -c "<script>"` wrapper** — its reads are invisible,
   but the script text is already a hash input, and `$VAR` references in that
   one-liner are statically detectable. No `$` references → the shell adds no
   env dependencies of its own.
3. **Anything else** (python, go, static tools, compiled formatters…) —
   unobservable → mark the task unverifiable → fall back to declared `env`
   exactly as today (or optionally warn).

Within the verifiable subset, value-keyed traces are sound. The honest costs:

- **Hit-rate erosion from housekeeping reads.** Node internals read
  `FORCE_COLOR`, `TERM`, `NODE_OPTIONS`, etc. (Experiments 2/5); a trace
  keyed on their values misses across dev/CI boundaries. A curated ignore
  list fixes hit rates but is a deliberate, documented unsoundness hole
  (colored output *can* end up in artifacts). This is a judgment call, not a
  correctness proof.
- **Cache plumbing changes shape.** Today's model is `hash → artifact`;
  traces need `static-hash → [(read-set, values) → artifact]` with
  get→validate→maybe-refetch rounds, including in the remote cache protocol
  and its HTTP API.
- **Ecosystem fragility.** The npm silent-death bug (Experiment 3) is the
  cautionary tale: the recorder sits under every user process, and its edge
  cases become turbo bugs. Windows needs a separate mechanism for the
  non-Node census (no LD_PRELOAD; Node-level spawn hooks cover part of it).
- **Determinism is assumed, not enforced** — same as turbo's existing model.

**Bottom line for caching:** a static learned list can never be trusted in
the hash; a value-keyed trace with spawn-time escape detection *can* be made
sound for Node-only task trees, at the price of a new cache-entry model,
per-platform census machinery, and a curated housekeeping ignore list. The
suggestion/strict-warning mode from Part 1 remains the low-risk first step
and shares all of its machinery with this design.

## Part 3 — Static analysis: can a linter see every env var that impacts the program?

Short answer: **no linter can be complete, even in principle — but not for the
reason you'd guess.** The hard ceiling isn't exotic metaprogramming in user
code; it's that most env reads in a real app don't happen in user code at all.

### Experiment 7 — the shipped `eslint-plugin-turbo` vs. a 12-pattern corpus

The existing rule (`no-undeclared-env-vars`) AST-matches three shapes —
`process.env.X`, `process.env["X"]`, and destructuring — plus framework
wildcard allowlists via dependency inference. We ran the *published* plugin
against `lint-corpus.js`, twelve realistic access patterns each reading a
distinct variable:

| # | Pattern | Caught? | Fixable statically? |
|---|---|---|---|
| 1 | `process.env.V01` | ✅ | — |
| 2 | `process.env["V02"]` | ✅ | — |
| 3 | `const { V03 } = process.env` | ✅ | — |
| 4 | `const { V04: renamed } = process.env` | ✅ | — |
| 5 | `const e = process.env; e.V05` | ❌ | needs data-flow analysis |
| 6 | `process.env["V06_" + "CONCAT"]` | ❌ | yes — constant folding |
| 7 | ``process.env[`V07_${"TPL"}`]`` | ❌ | yes — constant folding |
| 8 | `getEnv("V08")` helper indirection | ❌ | inter-procedural data flow; hard cross-module |
| 9 | `Reflect.get(process.env, "V09")` | ❌ | yes — one more AST shape |
| 10 | `"V10" in process.env` | ❌ | **yes — cheap rule fix, worth doing regardless** |
| 11 | `Object.keys(process.env).filter(k => k.startsWith("V11_"))` | ❌ | no finite list exists; only expressible as a `V11_*` wildcard |
| 12 | same shapes in another linted module | ✅ | — |

5 of 12 caught. Several misses are engineering (6, 7, 9, 10 are
mechanical rule improvements; 5 and 8 need real data-flow analysis, e.g. a
TS-program-based rule). Pattern 11 is the theoretical wall: a prefix scan has
no finite variable list — though turbo's `env` already supports wildcards, so
a linter could *suggest* `V11_*`. This is the classic static-analysis
trade-off: to be sound it must over-approximate (whole-env wildcards), to be
precise it must under-approximate (miss dynamic reads). It cannot do both.

### Experiment 8 — the dependency iceberg (why completeness is dead on arrival)

Linters lint *your* source. Env vars are read by *the whole program*:

- The 73-package `node_modules` of a bare eslint toolchain contains
  **35 distinct literal env var reads** (`DEBUG`, `NODE_ENV`, `TIMING`,
  `HTTP(S)_PROXY`, `DOTENV_CONFIG_*`, …) plus 2 dynamic `process.env[t]`
  sites — none of it ever linted.
- `next/dist` alone reads **303 distinct literal env vars** and has **19
  dynamic access sites** (`process.env[key]`, `process.env[innerKey]`, …),
  through aliases like `const o = process.env` that defeat even text-level
  scans. Every Next.js app "reads" env vars its author has never heard of.

Linting `node_modules` isn't the fix: it's minified, aliased, dynamic, and
the sheer volume (303 vars for one framework) would drown the signal.
Turbo's `frameworks.json` inference is the existing — and correct — curated
workaround for exactly this layer.

### What a linter *is* good for

1. **Authoring-time suggestions in user code** — the current rule, upgraded
   with constant folding, `in`-check support (#10 above), `Reflect.get`, and
   optionally TS-based alias tracking. High value, no runtime risk, and it
   sees code paths that never executed — the one thing runtime tracing can't.
2. **A soundness classifier for the caching design in Part 2**: don't try to
   list the vars; instead classify each workspace's own code as "all env
   accesses statically enumerable" vs. "contains unanalyzable access → bail."
   That's the static twin of Experiment 6's spawn-time escape detection. But
   it only classifies *user* code — the dependency iceberg means it must be
   paired with runtime traces (which see dependency reads perfectly, since
   they observe the actual lookups regardless of which package makes them).
3. **Shell one-liners**: `$VAR` references in `package.json` script text are
   trivially parseable, closing the gap runtime tracing proved blind to
   (Experiment 4's `DOCS_TOKEN`).

**Division of labor that actually covers the space:** framework layer →
curated inference (exists); user source → linter (exists; several mechanical
upgrades identified above); dependencies + dynamic keys → runtime traces
(Parts 1–2); shell wrappers → script-text parsing. Any single layer alone —
including a maximally smart linter — is provably incomplete as a cache-hash
source of truth.

## Files

- `shim.c` — LD_PRELOAD getenv/secure_getenv interposer (Experiments 2, 4)
- `audit-node.cjs` — `process.env` Proxy recorder, with the npm-safe set trap
  and re-entrancy guard (Experiments 3–5)
- `readers/` — per-runtime test programs (C, Go, Rust, conditional-access JS)
- `fixture/` — two-workspace turbo monorepo used in Experiment 4
  (`npm install turbo` inside it, then run with the env vars shown above)
- `execsnoop.c` — LD_PRELOAD process-tree census: exec interposition + self-announce (Experiment 6)
- `lint-corpus.js` — 12 env-access patterns for testing static analyzers (Experiment 7)
- `run-matrix.sh` — one-command reproduction of the Experiment 2 matrix
