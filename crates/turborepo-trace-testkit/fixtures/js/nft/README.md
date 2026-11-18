# nft parity fixtures

The unit test fixtures of [`@vercel/nft`](https://github.com/vercel/nft), vendored as the M2
acceptance suite for JS tracing (design §2, §11, §13). The harness is
`crates/turborepo-trace-lang-js/tests/nft_parity.rs`.

## Attribution and license

Everything in this directory except `README.md` and `baseline.json` is copied unmodified from
nft (Copyright 2019 Vercel, Inc., MIT license, see `_root/LICENSE`):

| upstream | here |
|---|---|
| `test/unit/<case>/` | `<case>/` (155 cases, symlinks preserved via `git archive`) |
| `LICENSE`, `package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `.npmrc` | `_root/` |
| `test/unit.test.js` (the runner the harness ports) | `_root/unit.test.js` |

Pinned commit: `7d6e6e482e282663a59cbfabff94c73d2caec29f` (2026-08-18), also recorded in
`_root/UPSTREAM`. Re-vendored with turbotrace's `nft-parity` tooling; after a re-vendor, diff
`_root/unit.test.js` against `crates/turborepo-trace-lang-js/tests/nft_parity/cases.rs`.

## Running

```sh
cargo test -p turborepo-trace-lang-js --test nft_parity -- --nocapture
```

Cases that need nft's npm dev dependencies are skipped unless they are installed
(`NFT_PARITY_ROOT`, below); turbotrace's `nft-parity` tooling installs them.

Outputs: a Markdown table on stdout and in `target/nft-parity/summary.md`, plus
`target/nft-parity/report.json` (per case: status, missing, extra, sub-checks, options,
outside-base files, uncertainty count, timing).

Environment variables: `NFT_PARITY_UPDATE_BASELINE=1`, `NFT_PARITY_FILTER=a,b` (substring
filter), `NFT_PARITY_REQUIRE_DEPS=1`, `NFT_PARITY_ROOT=<dir>`.

The test is a report. It fails only when a case listed in `baseline.json` stops passing.
Baseline cases that are skipped, for example because the npm deps are missing, do not fail it.

## Harness semantics (port of `unit.test.js`)

- Layout. nft runs with `base` set to its repo root and `processCwd` set to the case
  directory. Expected paths therefore look like `package.json`, `test/unit/<case>/...` and
  `node_modules/<pkg>/...`. The harness rebuilds that tree under `target/nft-parity/root/`:
  `package.json` from `_root/`, a fresh copy of every case in `test/unit/` on each run, and
  `node_modules/` from `install`. It lives in `target/` rather than a gitignored
  `fixtures/js/nft/.deps/` because a sync to a remote checkout deletes untracked files outside
  `target/`.
- Entries follow nft: `input.js`, except `input.jsx`/`input.tsx`/`input.ts` for
  `jsx-input`/`tsx-input`/`ts-input-esm`, `input.mjs` for the five
  `module-create-require-*` cases, and four inputs for `multi-input`.
- Options:
  - `conditions` from `test-opts.json`. nft always adds `default` plus `import`/`require`, and
    adds `node` only when the option is unset. The harness passes
    `conditions + [import, require, default]`, because `--conditions` replaces turbotrace's
    whole set.
  - `exportsOnly` for `exports-only*`, via `JsResolveOptions::exports_only` in a test-only
    `Language` wrapper.
  - nft's `paths` aliases (`dep`, `dep/`), via a resolver wrapper. They are used only by
    `esm-paths*`.
  - `ignore` is `actual.js`.
  - Every other case uses `JsLanguage` unchanged, exactly as the CLI does.
- Two runs per case, like nft's `doTrace(true)`. A second trace runs on the same `Tracer`, so
  the caches are warm, and must equal the cold one; otherwise the case is an ERROR.
  `processed-dependency` uses `input-cached.js`/`output-cached.js` for the second run.
- Normalizations:
  - Node-gyp cases (`datadog-pprof-node-gyp`, `microtime-node-gyp`, `zeromq-node-gyp`): drop
    `prebuilds/**/*.node` and require at least one. The requirement shows up as the
    `<any prebuilds/**/*.node>` sentinel.
  - `webpack-wrapper-strs-namespaces-large`: hoisted vs nested `react-is` are treated as the
    same file.
- `multi-input` also checks nft's per-entry closures (`closure:<entry>` sub-checks) through
  `Trace::entry_files`.
- Only the `cwd` variant runs. nft also repeats every case with `base: '/'`, which is the
  same file set under an absolute prefix.
- Classification:
  - PASS: identical sets.
  - SUPERSET: only extra files.
  - MISSING: at least one missing file. Extra files may also be listed.
  - ERROR: build or trace error, panic, or a cold/warm mismatch.
  - SKIP: not run.

  A case gets the worst status of its sub-checks.

### Skipped cases (3 of 155)

| case | why |
|---|---|
| `module-sync-condition-es-node20`, `module-sync-condition-cjs-node20` | nft skips them on Node >= 22. Their expectations encode Node 20, which has no `module-sync` condition. We follow current Node |
| `resolve-hook` | tests nft's custom `resolve` + `readFile` API hooks with synthetic `custom-resolution-*` ids. turbotrace has no equivalent |

### Cases that need nft's npm deps (12)

`asset-fs-extra`, `asset-graceful-fs`, `browserify-minify`, `datadog-pprof-node-gyp`,
`microtime-node-gyp`, `phantomjs-prebuilt`, `pixelmatch`, `pkginfo`, `resolve-from`,
`shiki`, `webpack-wrapper-strs-namespaces-large`, `zeromq-node-gyp`.

The harness detects them from `node_modules/...` entries in `output.js`. They are skipped with
`requires-npm` unless nft's dev dependencies are installed. The install mirrors nft's CI:
`pnpm install --frozen-lockfile` with `node-linker=hoisted`, plus `--ignore-scripts`. nft's
`allowBuilds` disables install scripts for all of these packages anyway. Node-gyp prebuilds
ship in the package tarballs.

### Options the harness cannot express (the cases still run and are reported)

| option | cases |
|---|---|
| `depth` | `depth-0` ... `depth-3` |
| `analysis: false` | `basic-analysis-require` |
| `mixedModules: false` | `module-create-require-{destructure,ignore-other,named-import,named-require,no-mixed}` |
| `moduleSyncCatchall` | `module-sync-catchall` |

## Gap analysis, snapshot 1: M1 HEAD `db80cde`

Result: 54 PASS, 4 SUPERSET, 94 MISSING, 0 ERROR, 3 SKIP. Runtime is 0.46 s for all cases,
cold and warm, in a debug build. Owners are the parallel M2 workstreams: `m2-eval` covers JS
partial evaluation and fs/URL/Worker/createRequire API tracking; `m2-trace` covers globs and
patterns, special cases, sharedlibs and manifests. A case can appear in more than one row.

| # | root cause | cases | owner |
|---|---|---|---|
| 1 | `__dirname`/`path.join`/`path.resolve`/concat/template evaluation for fs read APIs, including interop wrappers (babel `_interopRequireDefault`, TS `(0, path_1.join)`, `node:` prefix, ESM default import, shadowing) and `process.cwd()` | `array-emission`, `asset-fs-extra`, `asset-fs-inline-path-{babel,enc-es,enc-es-2..5,enc-es-node-prefix,shadow,ts,ts-no-interop}`, `asset-fs-inline-tpl`, `asset-fs-inlining`, `asset-fs-inlining-multi`, `asset-graceful-fs`, `class-static`, `fs-emission` (variable `asset3`), `jsx-input` (dep.jsx), `path-sep`, `ts-path-join`, `ts-filter`, `multi-input` (assets), `depth-2`/`depth-3` (assets), `datadog-pprof-node-gyp` (mappings.wasm), `ffmpeg-installer` | m2-eval |
| 2 | Asset emission of `__dirname`-rooted path values that never reach an fs call: assigned, returned, or passed to `spawn` | `asset-symlink`, `asset-conditional`, `asset-fs-array-expr`, `asset-fs-array-expr-node-prefix`, `return-emission` | m2-eval |
| 3 | `new URL(x, import.meta.url)` / `fileURLToPath` | `import-meta-url`, `import-meta-tpl-cnd` | m2-eval |
| 4 | Conditional / alternative values (`?:`, `\|\|`, env-dependent branches) | `asset-fs-logical`, `import-meta-tpl-cnd`, `process-env`, `require-var-branch`, `require-dynamic-fallback`, `mongoose` | m2-eval |
| 5 | Computed `require` evaluated to a literal or a pattern: template with `__dirname`, `.concat`, loop or array variables, require wrappers (a function parameter passed to `require`) | `require-dirname-tpl`, `string-concat`, `require-wrapper`, `require-wrapper2`, `require-wrapper3`, `protobuf-loop`, `protobuf-loop2`, `webpack-node`, `resolve-from` (`resolveFrom(__dirname, x)`) | m2-eval |
| 6 | Pattern/glob and directory expansion (`readdir(__dirname)`, `unknown` → wildcard, dotfiles) | `wildcard`, `wildcard2`, `wildcard3`, `wildcard-require`, `glob-dot`, `dirname-emit`, `dirname-emit-concat`, `process-cwd` | m2-eval produces the pattern; m2-trace expands it |
| 7 | Bundler wrappers (webpack runtime/chunk tables, browserify bundles) | `browserify-minify`, `webpack-wrapper`, `webpack-wrapper-multi`, `webpack-wrapper-dirname-inject`, `webpack-5-wrapper-namespace`, `webpack-wrapper-strs-namespaces`, `webpack-wrapper-strs-namespaces-large` | m2-eval |
| 8 | Package special cases and native binaries: node-gyp-build prebuilds, phantomjs, pixelmatch `bin/`, prisma engines, shiki languages/themes + onig.wasm, pino transports | `datadog-pprof-node-gyp`, `microtime-node-gyp`, `zeromq-node-gyp`, `phantomjs-prebuilt`, `pixelmatch`, `prisma-photon`, `shiki`, `pino-transport`, `pino-transport-constructor`, `pino-transport-fastify`, `pino-transport-targets` | m2-trace |
| 9 | `module.createRequire(import.meta.url / new URL(...))` tracking, and a shadowed local `require` that must be ignored | `module-create-require`, `module-create-require-named-import`, `module-create-require-named-require`, `module-create-require-no-mixed` (missing `sub/lib.node`), `module-create-require-ignore-other` (we emit `lib.node`, SUPERSET) | m2-eval |
| 10 | `module.register(specifier, parentURL)` hooks | `module-register` | m2-eval (API tracking) |
| 11 | Resolver conditions: Node >= 22 `module-sync` condition, `browser` field remapping when `browser` is a condition | `module-sync-condition-{cjs,es,es-default,es-nested}`, `imports-module-sync`, `imports-module-sync-cjs`, `self-reference-module-sync`, `browser-remappings` | resolver, not assigned (closest: m2-trace) |
| 12 | `package.json` emission (a): the entry's package scope is emitted only when some reference resolves. nft always emits it for `.js` files. Only difference in 7 cases; contributes to 12 more | `array-holes`, `dirname-len`, `null-destructure`, `require-call`, `require-empty`, `syntax-err`, `url-error` (+ `mongoose`, `process-env`, `require-*`, `return-emission`, `string-concat`, `wildcard-require`, `browserify-minify`) | m2-trace (manifests) |
| 13 | `package.json` emission (b): we also emit the scope `package.json` for `.mjs`/`.jsx`/`.tsx` files. nft does not, because the extension fixes the module type | `jsx-input`, `tsx-input` (SUPERSET), `module-create-require-{named-import,named-require,no-mixed,ignore-other}` | candidate intentional difference (over-approximation); decide in m2-trace |
| 14 | Unsupported nft options | `depth-0`, `depth-1` (SUPERSET: no depth limit), `depth-2`, `depth-3`, `module-sync-catchall`, `mixedModules=false` variants | intentional unless we add the options |

Intentional differences so far: rows 13 (pending a decision) and 14. Our stricter type-only
exclusion has not caused a difference in any case yet.

## Gap analysis, snapshot 2: uncommitted M2 work in progress (m2-eval + m2-trace), 2026-11-02

Result: 87 PASS, 11 SUPERSET, 54 MISSING, 0 ERROR, 3 SKIP. 33 cases pass that did not pass at
M1, none of the 54 baseline cases regressed, and the run takes 0.47 s. `baseline.json` is
still the M1 set; update it when that work lands. The table below lists what remains, using the
row numbers from snapshot 1.

| # | still open | fixed since snapshot 1 |
|---|---|---|
| 1 | `asset-fs-extra` (`${process.cwd()}/...`), `asset-fs-inline-path-babel`, `asset-fs-inline-path-ts`, `asset-fs-inline-path-ts-no-interop`, `asset-fs-inline-tpl`, `asset-graceful-fs`, `path-sep`, `ts-path-join`, `ts-filter`, `ffmpeg-installer`, `datadog-pprof-node-gyp` (mappings.wasm) | `array-emission`, `asset-fs-inline-path-enc-es*`, `asset-fs-inline-path-shadow`, `asset-fs-inlining*`, `class-static`, `multi-input`, `jsx-input` (dep.jsx), depth assets, `fs-emission` asset3 |
| 2 | all 5 (`asset-symlink`, `asset-conditional`, `asset-fs-array-expr*`, `return-emission`) | none |
| 3 | `import-meta-tpl-cnd` | `import-meta-url` |
| 4 | `asset-fs-logical`, `import-meta-tpl-cnd`, `require-var-branch` | `process-env`, `require-dynamic-fallback`, `mongoose` |
| 5 | `require-dirname-tpl`, `require-wrapper`, `require-wrapper2`, `require-wrapper3`, `protobuf-loop`, `protobuf-loop2`, `resolve-from` | `string-concat`, `webpack-node` |
| 6 | `wildcard`, `wildcard2`, `wildcard3`, `glob-dot`, `dirname-emit`, `process-cwd` | `wildcard-require`, `dirname-emit-concat` |
| 7 | all 7 bundler-wrapper cases | none |
| 8 | `datadog-pprof-node-gyp` (prebuild), `pino-transport*` (4), `shiki` (only `onig.wasm` left) | `microtime-node-gyp`, `zeromq-node-gyp`, `phantomjs-prebuilt`, `pixelmatch`, `prisma-photon` |
| 9 | `module-create-require-ignore-other` (still emits `lib.node`) | `module-create-require`. `-named-*` and `-no-mixed` now only differ by row 13 |
| 10 | `module-register` | none |
| 11 | all 8 (`module-sync` condition, browser field) | none |
| 12 | none | all |
| 13 | `jsx-input`, `tsx-input`, `module-create-require-{named-import,named-require,no-mixed,ignore-other}` (SUPERSET) | none |
| 14 | `depth-0` ... `depth-3` (SUPERSET), `module-sync-catchall` | none |
| new | `fs-emission` (SUPERSET): we now emit the cwd-relative literal `fs.readFile('./asset1.txt')` resolved against `processCwd`. nft does not | candidate intentional over-approximation (m2-eval) |

## Gap analysis, snapshot 3: nft parity work (nft-gaps), 2026-11-03

Result: 140 PASS, 11 SUPERSET, 1 MISSING, 0 ERROR, 3 SKIP (from 87 / 11 / 54 / 0 / 3), with
deps installed, in about 0.5 s. No baseline case regressed; `baseline.json` now lists the 140
passing cases. Each fix has a focused fixture: `fixtures/js/resolve/nft-{assets,loaders,conditions}/`
(`crates/turborepo-trace-lang-js/tests/nft_gaps.rs`) and unit tests in `crates/turborepo-trace-lang-js/src/extract/tests.rs`.

| root cause | fix | cases fixed |
|---|---|---|
| Path values that never reach a tracked fs call (rows 1, 2, 6) | The extractor's deep pass (`extract/deep.rs`) re-lexes files that mention `__dirname`, `__filename` or `process.cwd` and emits the largest enclosing expression that evaluates to an anchored path as an `asset`: assigned, returned, exported, in a `spawn` array, read through an undeclared global `fs`, or bound to a top-level or function-local name used later. Own directory, cwd, ancestors and the file itself are never emitted; a directory value expands with a `partial_specifier` record | `asset-conditional`, `asset-fs-array-expr`, `asset-fs-array-expr-node-prefix`, `asset-symlink`, `return-emission`, `ts-path-join`, `path-sep`, `ts-filter`, `asset-fs-inline-path-babel`, `asset-fs-inline-path-ts`, `asset-fs-inline-path-ts-no-interop`, `wildcard`, `wildcard2`, `wildcard3`, `protobuf-loop`, `protobuf-loop2`, `ffmpeg-installer`, `process-cwd`, `datadog-pprof-node-gyp` (`mappings.wasm`) |
| Evaluator gaps (rows 1, 3, 4, 5) | A template whose first part is a substitution (`` `${__dirname}/x` ``, `` `${import.meta.url}/../x` ``); spread arguments as wildcards; top-level `let`/`var` initial values (with a record); inline `require('path'|'url')`; `resolve-from`; `require.resolve('pkg')` as an anchor for `path.join(..., '../x.wasm')`; `const x = require('m').f(...)` is a value, not a binding | `asset-fs-inline-tpl`, `asset-fs-logical`, `asset-fs-extra`, `asset-graceful-fs`, `import-meta-tpl-cnd`, `require-dirname-tpl`, `require-var-branch`, `resolve-from`, `shiki` |
| Bundler wrappers (row 7) | `X.join(anchor, ...)` / `X.resolve(anchor, ...)` on an unknown `X` (webpack's `path__WEBPACK_IMPORTED_MODULE_0___default()`, `.a`) is taken as `path.join` / `path.resolve`, with a record; browserify module tables (`[function (r, module, exports) {...}, {deps}]`): `r('x')` requires `x` unless the map points it at a bundled id (`aliased_loader`) | `webpack-wrapper`, `webpack-wrapper-multi`, `webpack-wrapper-dirname-inject`, `webpack-5-wrapper-namespace`, `webpack-wrapper-strs-namespaces`, `webpack-wrapper-strs-namespaces-large`, `browserify-minify` |
| Require wrappers (row 5) | `const load = name => require(name)` / `function load(name) { ... require(name) }`: `load('./x')` is a `require` (`aliased_loader`) | `require-wrapper`, `require-wrapper2`, `require-wrapper3` |
| pino transports (row 8) | In files importing `pino` or `fastify`, `target: 'module'` properties are optional requires (`aliased_loader`) | `pino-transport`, `pino-transport-constructor`, `pino-transport-fastify`, `pino-transport-targets` |
| node-gyp-build prebuilds (row 8) | Also glob prebuildify's libc-tagged directories (`prebuilds/linuxglibc-x64`) | `datadog-pprof-node-gyp` |
| `module-sync` and `browser` (row 11) | When the mapping manifest has `module-sync`, resolve once more with the condition toggled and keep both results; with the `browser` condition, add the `browser` field remapping of each target | `module-sync-condition-{cjs,es,es-default,es-nested}`, `imports-module-sync`, `imports-module-sync-cjs`, `self-reference-module-sync`, `browser-remappings` |
| `readdirSync(__dirname)` when the cwd is that directory (row 6) | `AssetRules::invalid_dir` admits the own directory for `FsReadDir` even when it is also the cwd | `dirname-emit`, `glob-dot` |
| `module.register` (row 10) | `register(spec, parentURL)` and `register(spec, { parentURL })` are ESM `import` references, resolved from the file or, for `pathToFileURL('./')`, the cwd (`cwd_dependent`) | 4 of the 5 files of `module-register` |

### Remaining gaps

| case | status | why |
|---|---|---|
| `module-register` | MISSING `hook3.mjs` | `register('./test/unit/module-register/hook3.mjs', pathToFileURL('./'))` is relative to the cwd. nft evaluates `pathToFileURL('./')` against its own process cwd (the nft repo root while its tests run); the harness's `processCwd` is the case directory, where the path does not exist. We resolve it against the configured cwd and report `cwd_dependent` |
| `depth-0` ... `depth-3` | SUPERSET | nft's `depth` option is not supported (no depth limit) |
| `module-create-require-{ignore-other,named-import,named-require,no-mixed}` | SUPERSET | nft's `mixedModules: false` is not supported (`ignore-other` also emits `lib.node` through a plain `require`), plus the `.mjs` `package.json` difference below |
| `module-sync-catchall` | PASS by chance | nft's `moduleSyncCatchall` option is not supported; the default dual resolution already covers the expected files |

### Intentional differences

| difference | cases | why we keep it |
|---|---|---|
| We emit the scope `package.json` of `.mjs`, `.jsx` and `.tsx` files; nft does not, because for Node the extension fixes the module type | `jsx-input`, `tsx-input`, `module-create-require-*` (SUPERSET) | The scope manifest is still read at runtime for `imports` (`#internal`) and package self-references, and TS/JSX runners (tsx, ts-node, bundlers) read `type` for `.tsx`/`.jsx`. One small file per package scope; over-approximating is the safe choice (design §1) |
| A cwd-relative literal read (`fs.readFile('./asset1.txt')`) resolves against `--cwd` with a `cwd_dependent` record; nft ignores relative literals without `process.cwd()` | `fs-emission` (SUPERSET) | Node resolves it against the cwd, so a deployment run from that directory needs the file; the record says the result depends on `--cwd` |
| Guessed `X.join(anchor, ...)` as `path.join`, top-level and function-local `let`/`var` values, and directory values expanded as a whole all carry `partial_specifier` (or `cwd_dependent`) records; nft applies the same rules silently | (no file difference) | Every heuristic that can guess wrong is reported in the trace's uncertainties |
