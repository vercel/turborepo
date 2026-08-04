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

## Files

- `shim.c` — LD_PRELOAD getenv/secure_getenv interposer (Experiments 2, 4)
- `audit-node.cjs` — `process.env` Proxy recorder, with the npm-safe set trap
  and re-entrancy guard (Experiments 3–5)
- `readers/` — per-runtime test programs (C, Go, Rust, conditional-access JS)
- `fixture/` — two-workspace turbo monorepo used in Experiment 4
  (`npm install turbo` inside it, then run with the env vars shown above)
- `run-matrix.sh` — one-command reproduction of the Experiment 2 matrix
