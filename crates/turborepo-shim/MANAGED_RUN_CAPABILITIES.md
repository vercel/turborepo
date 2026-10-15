# Native managed-run capability selection

Schema readability is not execution support. The public `managedRunSchemas` list remains empty; the experimental Node/pnpm object must never populate it from the lock parser's schema version.

The standalone internal query is `--__internal-managed-run-capabilities`. Readers enforce the existing 4096-byte bound, query deadline, strict JSON/ABI validation, quiet stderr, successful exit, and owned-process cleanup.

| Response key | Meaning |
| --- | --- |
| `abiVersion` | Native query ABI, currently `1` |
| `cliVersion` | Diagnostic only; never a compatibility floor |
| `managedRunSchemas` | Complete public managed-run contract; currently `[]` |
| `experimentalNodePnpm.contractVersion` | Internal adapter/platform-bounded contract, currently `1` |
| `experimentalNodePnpm.lockSchema` | Explicit prelaunch lock schema, currently `0` |
| `experimentalNodePnpm.platform` | `macos-arm64` or `linux-x64-gnu` |

The experimental object is absent on every other host. GNU Linux ARM64, macOS x64, Linux musl, Windows, and other architectures remain unadvertised. A peer reporting another host, contract version, or schema cannot satisfy this host's experimental check. Old strict readers reject the extension rather than mistaking it for complete-schema support.

## Authorization remains required

Consumers must first authorize the exact-root managed Node/pnpm cohort using current declarations, complete installation resources and exports, native execution context, and matching private setup evidence. Experimental peer support does not bypass source policy, lock validation, trust checks, or revalidation at managed execution boundaries.

Installed incompatible, malformed, damaged, or unresolvable local pins fail closed rather than being substituted with the global CLI or implicitly fetched. Self-local and copied-local execution remain subject to the same capability requirement.

This object does not claim npm/bundled npm, Rust, Python, Go, verify-system adapters, or complete product-schema support.

## Binary-level journey

The companion setup/run journey in `crates/turborepo/tests/setup_run_journey.rs` is gated to macOS ARM64 and GNU Linux x64. It exercises global setup followed by plain run, absent and incompatible local pins, copied-local compatible handoff, strict and loose task environments, drift, damaged installations, and no-network reuse. This bounded journey is not qualification of the complete public managed-run schema or other hosts.
