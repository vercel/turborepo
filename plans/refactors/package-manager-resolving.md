# Package manager resolving refactor

Notes on a future refactor motivated by [nub](https://nub.dev) support and the
`PackageManager::Nub { lockfile }` wrapper introduced in #13120.

## Problem

`PackageManager` currently conflates several concerns:

1. **Declared identity** — what the user set in `package.json` (`packageManager`)
2. **CLI execution** — which binary to invoke (`npm`, `pnpm`, `nub`, …)
3. **Lockfile semantics** — parsing, pruning, patch handling, cache hashing
4. **Workspace discovery** — globs, `pnpm-workspace.yaml`, default exclusions

Every supported manager except nub maps 1:1 across these axes. nub splits them:

- Identity / CLI: `nub`
- Lockfile: whatever is already on disk (npm, pnpm, yarn, bun)

The current fix is a **wrapper variant** on the existing enum:

```rust
Nub {
    lockfile: Box<PackageManager>, // concrete backend, never nested Nub
}
```

This works but leaks complexity: call sites must know whether to use the outer
identity or the inner lockfile backend, and behavior routing is inconsistent
(some methods delegate, some special-case `Nub`, some use `lockfile_manager()`).

## Current implementation (as of nub integration)

### Detection

- nub is recognized **only** from declaration fields, never from lockfile alone.
- On detection, `underlying_lockfile_manager(repo_root)` probes disk:
  bun → pnpm → yarn → npm (default).
- Yarn Berry and pnpm6/9 are distinguished by lockfile contents, not just
  filename.

### Helpers introduced to contain leakage

| Helper                                  | Purpose                                                        |
| --------------------------------------- | -------------------------------------------------------------- |
| `lockfile_manager()`                    | Peel `Nub` to the concrete lockfile backend                    |
| `is_pnpm_family()`                      | Predicate for pnpm lockfile semantics via `lockfile_manager()` |
| `with_resolved_nub_lockfile(repo_root)` | Re-probe disk after daemon proto round-trip                    |

### Delegation vs nub-specific behavior

| Operation                                                   | Route                                             |
| ----------------------------------------------------------- | ------------------------------------------------- |
| `command()`, `name()`                                       | Outer nub identity                                |
| `read_lockfile`, `parse_lockfile`, `prune_patched_packages` | Delegate to inner `lockfile`                      |
| `read_catalogs`, `is_pnpm_family` (external)                | Via `lockfile_manager()`                          |
| `arg_separator`                                             | Outer nub (pnpm-compatible CLI)                   |
| `get_default_exclusions`                                    | Outer nub (npm-style)                             |
| `get_configured_workspace_globs`                            | Hybrid: pnpm-workspace.yaml if underlying is pnpm |

### Known limitations of the wrapper approach

- **Match-arm proliferation** — easy to miss a `Nub` arm when adding new
  `PackageManager` behavior.
- **Snapshot state** — `lockfile` is resolved at detection time; not automatically
  refreshed if lockfiles change without re-discovery.
- **Proto wire format** — daemon carries `Nub = 7` only; underlying type is lost
  on the wire and must be re-resolved from disk on the client.
- **Recursive type** — `Box<PackageManager>` inside `PackageManager` is a smell
  that the enum is doing composition without a composition model.
- **`supported_managers()`** — intentionally excludes nub (no lockfile of its
  own); lockfile change detection relies on iterating known lockfile names.

## Proposed target model

Split identity from lockfile backend explicitly:

```rust
struct ResolvedPackageManager {
    /// What the user declared and what we execute.
    identity: PackageManagerIdentity,

    /// Always concrete: Npm | Pnpm | Pnpm6 | Pnpm9 | Yarn | Berry | Bun.
    /// Never Nub.
    lockfile_backend: LockfileBackend,
}

enum PackageManagerIdentity {
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Nub,
    // ...
}
```

Or a trait-based split:

```rust
trait TaskExecutor {
    fn binary(&self) -> &str;
    fn arg_separator(&self, user_args: &[impl AsRef<str>]) -> Option<&str>;
}

trait LockfileProvider {
    fn read_lockfile(&self, root: &AbsoluteSystemPath, pkg: &PackageJson)
        -> Result<Box<dyn Lockfile>, Error>;
    fn lockfile_name(&self) -> &str;
}

trait WorkspaceDiscoverer {
    fn get_workspace_globs(&self, root: &AbsoluteSystemPath)
        -> Result<WorkspaceGlobs, Error>;
}
```

nub would implement `TaskExecutor` as nub and `LockfileProvider` /
`WorkspaceDiscoverer` by forwarding to `lockfile_backend`.

## Migration path

1. **Introduce `ResolvedPackageManager`** alongside the existing enum; populate
   both during detection. No call-site changes yet.
2. **Move lockfile methods** onto `LockfileBackend` (or `lockfile_backend` field
   accessors). Update `PackageGraph`, prune, cache hashing to use
   `resolved.lockfile_backend` instead of matching on `PackageManager`.
3. **Move execution methods** onto `identity`. Task executor uses
   `resolved.identity.binary()` only.
4. **Deprecate `PackageManager::Nub { lockfile }`** once all call sites use the
   split struct.
5. **Extend daemon proto** (or always re-resolve from disk at a single boundary)
   to carry `lockfile_backend` or document that identity-only wire values require
   disk re-probe.

## When to do this

- **Now (nub only):** wrapper + helpers is acceptable; cost is localized.
- **Trigger for refactor:** a second "facade" package manager with no native
  lockfile, or repeated bugs from missed `Nub` match arms / delegation gaps.
- **Do not block nub merge** on this refactor; treat it as follow-up technical
  debt with a clear migration sketch.

## Related files

- `crates/turborepo-repository/src/package_manager/mod.rs` — enum, helpers,
  delegation match arms
- `crates/turborepo-repository/src/package_manager/nub.rs` — underlying lockfile
  resolution
- `crates/turborepo-repository/src/package_graph/builder.rs` —
  `with_resolved_nub_lockfile` after discovery
- `crates/turborepo-lib/src/run/package_discovery/mod.rs` — daemon client
  re-resolution
- `crates/turborepo-daemon/src/proto/turbod.proto` — `Nub = 7` wire value
- `crates/turborepo-lib/src/commands/prune.rs` — `is_pnpm_family()` usage

## Open questions

- Should workspace discovery always follow `lockfile_backend`, or should nub have
  fixed opinions (current hybrid for pnpm-workspace.yaml)?
- Should the JS tooling (`packages/turbo-workspaces`) share a single
  `SUPPORTED_PACKAGE_MANAGERS` list with Rust via codegen or a shared JSON
  schema?
