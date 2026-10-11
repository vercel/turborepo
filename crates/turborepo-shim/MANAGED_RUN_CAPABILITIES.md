# Native managed-run capability selection

Schema readability is not execution support. The public complete-schema list remains empty for this CLI. Do not populate it from the lock parser's `SCHEMA_VERSION`.

The exact standalone internal query is `--__internal-managed-run-capabilities`. It runs before repository parsing, shim dispatch, downloads, telemetry, and agent guidance. Readers enforce the existing 4096-byte bound, three-second query deadline, strict JSON/ABI validation, quiet stderr, successful exit, and owned-process cleanup.

| Response key | Meaning in this build |
| --- | --- |
| `abiVersion` | Native query ABI, currently `1` |
| `cliVersion` | Diagnostic only; never a compatibility floor |
| `managedRunSchemas` | Complete public managed-run contract; currently `[]` |
| `experimentalNodePnpm.contractVersion` | Startup-only internal adapter ABI, currently `1`; not complete peer qualification |
| `experimentalNodePnpm.lockSchema` | Explicit prelaunch lock schema, currently `0` |
| `experimentalNodePnpm.platform` | Observed native startup host, initially `macos-arm64`; complete qualification pending |

The experimental object is absent outside the observed startup platform. Old strict readers reject this extension, rather than mistaking it for complete-schema support. A wrong contract version, schema, or platform cannot meet the internal startup ABI. This object never certifies the complete peer contract: version labels, schema parsing, synthetic peers and isolated startup results cannot provide that qualification.

## Scope and unsupported behavior

The caller must first authorize the exact-root managed Node/pnpm cohort: `experimentalSetup`, root `turbo.json` or `turbo.jsonc`, current declarations, complete installation resources/exports, native execution context, and matching private setup evidence. Experimental peer support never bypasses that authorization. Revalidation occurs after the query, before local dispatch, and again at the existing managed execution boundaries.

- Native macOS ARM64 Node/pnpm has bounded startup evidence only. Agent/proxy host-tool lookup, cache-hit validation and late-validation activation fixes remain pending; the combined result must be tested before advertising complete peer support, including on macOS.
- Do not merge the public schema `[0]` claim from partial startup commit `e66df9ddd0b9a5b064572c15a7823c856cb4f612`. The parent owns extraction of only its help/version fixes; this recovery owns the internal ABI and root forwarding.
- Linux GNU peer qualification is separately owned and is not advertised by this change. macOS x64, Linux musl, Windows, and other architectures are also unadvertised.
- npm/bundled npm, Rust, Python, Go, verify-system adapters, and the full product schema are not claimed by this experimental object.
- Self-local execution stays in-process, under the same capability requirement. A copied local binary is queried and dispatched through the existing child path.
- Installed incompatible, malformed, damaged, or unresolvable pins fail. They are not substituted with a global CLI or implicitly fetched.
- The resolved `--root-turbo-json` path crosses the local handoff as an absolute CLI argument before `--`; task arguments remain task arguments.

The lock wire schema itself is independently validated: `schemaVersion` and `tools`; each tool's `adapter`, `version`, `declarations`, optional `options`, and tagged `installation`; managed artifact `url`, `sha256`, `format`, optional `rootPrefix`/`destination`, and `executables`. Supporting those keys in the parser is not a claim that every installation kind, adapter, platform, or product-contract guarantee is executable here.

## Real current-build qualification

`native_handoff_tests.rs` is an explicitly opt-in process test. Supply real local Node distribution and pnpm 10.0.0 package roots and the binary built from the worktree under review. Fixture setup uses the existing owned loopback transport and production publication/authorization; run uses actual fresh CLI processes, not an injected RunBuilder environment.

For debug builds, optimize SHA-256 itself to keep repeated complete-tree verification within the fixture's 60-second per-process bound. This changes code generation, not validation, and is not a startup performance qualification.

```sh
cargo build --locked --offline --config 'profile.dev.package.sha2.opt-level=3' -p turbo
TURBO_TEST_BINARY=/absolute/path/to/current/turbo \
TURBO_TEST_NODE_ROOT=/absolute/path/to/node \
TURBO_TEST_PNPM_ROOT=/absolute/path/to/pnpm \
cargo test --locked --offline --config 'profile.dev.package.sha2.opt-level=3' -p turborepo-cli --lib \
  current_native_self_local_and_copied_local_startup_preserve_root_config \
  -- --ignored --nocapture --test-threads=1
```

The test keeps two distinct valid root files: flag-off `turbo.json` and explicitly selected managed `turbo.jsonc`. Setup and every positive run explicitly select the managed file. Without forwarding it, copied-local startup fails rather than discovering the same managed default. Self-local/copied-local strict and loose tasks, direct local invocation, nested invocation-relative selection, unchanged setup resources/lock, preserved task arguments, drift and damaged-pin rejection are checked. Host runtime sentinels refuse fallback. A no-forward mutation of the native CLI must fail this fixture. These are startup tests, not combined activation/probe/cache/late-validation, public-schema, Linux/Windows, or public setup transport qualification.
