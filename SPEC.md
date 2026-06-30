# Support devEngines.packageManager

## Summary

Turborepo should support `devEngines.packageManager` in the root `package.json` as a package manager declaration source. User-facing documentation, errors, and package-manager metadata tooling should lean into `devEngines.packageManager` as the preferred ecosystem-aligned declaration, while preserving existing top-level `packageManager` behavior for compatibility.

Implementation precedence remains:

1. Top-level `packageManager` if present.
2. `devEngines.packageManager` if top-level `packageManager` is absent.
3. Missing-declaration error if both fields are absent.

This is read-path support plus aligned tooling/docs updates. It must not change package-manager behavior after the declaration resolves to the existing internal `PackageManager` enum.

## Motivation

npm introduced `devEngines` to describe development-time tooling expectations in `package.json`. Supporting `devEngines.packageManager` lets Turborepo participate in that newer ecosystem standard while continuing to support existing repositories that use the top-level Corepack `packageManager` field.

The desired posture is:

- Prefer documenting `devEngines.packageManager` going forward.
- Keep top-level `packageManager` authoritative when it exists.
- Keep implementation compatible with current package-manager detection and downstream behavior.
- Avoid broad scope expansion into full npm `devEngines` enforcement.

## Package JSON Shape

Supported root `package.json` shape:

```json
{
  "devEngines": {
    "packageManager": {
      "name": "pnpm",
      "version": "9.12.3"
    }
  }
}
```

Supported `name` values:

- `npm`
- `pnpm`
- `yarn`
- `bun`

Rules:

- `devEngines.packageManager` must be an object.
- Arrays are not supported.
- `name` is required.
- `name` must be a lowercase string matching a supported package manager.
- `version` is required.
- `version` must be an exact valid semver version.
- Valid semver prerelease/build metadata is allowed if accepted by the existing semver parser.
- Version ranges are not supported.
- URL versions are not supported.
- Non-semver Corepack integrity strings are not supported.
- Leading/trailing whitespace in `name` or `version` is invalid.
- Empty strings are invalid values, not missing values.
- `onFail` is ignored entirely.
- Unknown properties are ignored for detection and preserved during serialization.

Examples of valid exact versions:

```json
{ "name": "pnpm", "version": "9.12.3" }
{ "name": "pnpm", "version": "9.12.3-alpha.0" }
{ "name": "yarn", "version": "4.5.0+sha224.abc" }
```

Examples of invalid versions:

```json
{ "name": "pnpm", "version": "^9.0.0" }
{ "name": "pnpm", "version": "9" }
{ "name": "pnpm", "version": "9.12" }
{ "name": "pnpm", "version": " 9.12.3 " }
{ "name": "pnpm", "version": "https://registry.npmjs.org/pnpm/-/pnpm-9.12.3.tgz" }
{ "name": "pnpm", "version": "9.12.3+sha512.Purxi/Zex==" }
```

## Detection Behavior

### Precedence

If top-level `packageManager` exists, Turborepo uses existing top-level behavior. `devEngines.packageManager` must not override it.

```json
{
  "packageManager": "pnpm@9.12.3",
  "devEngines": {
    "packageManager": { "name": "npm", "version": "10.5.0" }
  }
}
```

This resolves as `pnpm` because top-level `packageManager` is authoritative.

If top-level `packageManager` is absent and `devEngines.packageManager` exists, Turborepo parses and validates `devEngines.packageManager` and resolves it to the existing internal `PackageManager` enum.

If neither field exists, Turborepo should hard error as it does today for missing top-level `packageManager`. The user-facing message should recommend adding root `devEngines.packageManager` as the preferred declaration, with top-level `packageManager` mentioned only as legacy/backward-compatible support where useful.

### Root Only

Only the root `package.json` participates in this feature. Turborepo should not add upward parent-directory searching and should not read workspace package `devEngines.packageManager` fields for detection.

### Version Mapping

After parsing exact semver, mapping to internal variants must reuse existing behavior:

- `pnpm` versions map through the existing `PnpmDetector::detect_pnpm6_or_pnpm` logic.
- `yarn` versions map through the existing `YarnDetector::detect_berry_or_yarn` logic.
- `npm` maps to `PackageManager::Npm`.
- `bun` maps to `PackageManager::Bun`.

The version is transient. Do not store it in long-lived package graph state or add it to public APIs unless an existing API already exposes it for equivalent behavior.

### Lockfile Mismatch Validation

When `devEngines.packageManager` is used as the declaration source, Turborepo should validate it against existing implicit lockfile detection signals.

Current implicit detection signals are root lockfiles:

- `pnpm-lock.yaml`
- `package-lock.json`
- `yarn.lock`
- `bun.lock`
- binary-only `bun.lockb`, which currently surfaces the existing `BunBinaryLockfile` error

Do not expand mismatch validation to package-manager config files such as `pnpm-workspace.yaml` or `.yarnrc.yml` unless existing implicit detection starts using those files.

If implicit detection finds no lockfile signal, there is no mismatch. Return the declared manager.

If implicit detection finds a conflicting manager, hard error.

```json
{
  "devEngines": {
    "packageManager": { "name": "pnpm", "version": "9.12.3" }
  }
}
```

With `package-lock.json`, this is an error because the declaration says `pnpm` but the lockfile indicates `npm`.

If multiple lockfile signals exist, use existing implicit detection behavior. That means the existing multiple-package-manager error should still occur.

For mismatch comparison:

- Treat `Pnpm6`, `Pnpm`, and `Pnpm9` as the same pnpm family because current lockfile detection only proves `pnpm`.
- Keep Yarn classic and Berry distinct because current `yarn.lock` parsing distinguishes them.
- Keep `npm` and `bun` distinct.

When possible, mismatch diagnostics should include both the declared manager and the lockfile signal that caused the conflict.

### Malformed devEngines.packageManager

If top-level `packageManager` is absent and `devEngines.packageManager` exists but is malformed, detection must stop immediately with a hard error. Do not silently fall back to lockfile detection. The user expressed intent, but the intent is not properly expressed.

Hard-error cases include:

- `devEngines` is present but not an object.
- `devEngines.packageManager` is present but not an object.
- `devEngines.packageManager` is `null`.
- `devEngines.packageManager` is an array.
- `name` is missing.
- `name` is not a string.
- `name` is empty.
- `name` is unsupported.
- `version` is missing.
- `version` is not a string.
- `version` is empty.
- `version` is not exact valid semver.
- lockfile mismatch occurs after successful parsing.

Unsupported names should fail on `name` before version validation.

Declaration shape and semver validation should happen before lockfile mismatch validation.

For an empty object, prefer a combined shape error instead of reporting only missing `name` or only missing `version`.

```json
{
  "devEngines": {
    "packageManager": {}
  }
}
```

Diagnostic should communicate the expected shape, such as:

```json
{
  "devEngines": {
    "packageManager": {
      "name": "pnpm",
      "version": "9.12.3"
    }
  }
}
```

## dangerouslyDisablePackageManagerCheck

`dangerouslyDisablePackageManagerCheck` should apply to package manager declaration checks for both supported declaration fields.

When enabled:

- Bypass `devEngines.packageManager` parsing and mismatch validation.
- Use today's existing best-effort implicit detection behavior.
- Preserve existing implicit detection errors, such as multiple lockfiles or binary-only `bun.lockb`.

The option should not introduce a new default package manager. It should not suppress existing implicit detection consistency errors unless current behavior already does.

User-facing docs, comments, and schema descriptions for this option should mention both fields or use declaration-neutral wording such as "package manager declaration checks in root package.json".

## Data Model

Rust `PackageJson` parsing should add structured support for `devEngines` rather than reading it ad hoc from unstructured `other` data.

Recommended shape:

- `PackageJson.dev_engines: Option<DevEngines>`
- `DevEngines.package_manager: Option<DevEnginesPackageManager>`
- `DevEngines.other` to preserve non-package-manager entries.
- `DevEnginesPackageManager.name` with span metadata.
- `DevEnginesPackageManager.version` with span metadata.
- `DevEnginesPackageManager.other` to preserve ignored properties such as `onFail` and unknown future keys.

Spans are required for actionable diagnostics. Labels should point to the most specific location available:

- `devEngines.packageManager` for wrong type, null, array, or combined shape errors.
- `devEngines.packageManager.name` for missing, non-string, empty, unsupported, or mismatch errors.
- `devEngines.packageManager.version` for missing, non-string, empty, or invalid semver errors.

Round-trip serialization must preserve:

- Existing `devEngines` entries such as `runtime`, `cpu`, `os`, and `libc`.
- Unknown `devEngines` keys.
- `devEngines.packageManager.onFail`.
- Unknown `devEngines.packageManager` keys.

## TypeScript Tooling

TypeScript package-manager detection in `@turbo/workspaces` must support `devEngines.packageManager` with the same rules as Rust detection.

Requirements:

- Same precedence: top-level `packageManager`, then `devEngines.packageManager`, then existing implicit detection.
- Same supported names.
- Same exact-semver-only rule.
- Same array rejection.
- Same handling of malformed objects.
- Same ignored `onFail` and unknown properties.
- Same user-facing preference for `devEngines.packageManager` in errors.

Rust and TypeScript detection must not disagree on whether a root package manager declaration is valid.

## Codemod Behavior

The existing `add-package-manager` codemod should keep the same transformer name but write `devEngines.packageManager` instead of top-level `packageManager`.

New output shape:

```json
{
  "devEngines": {
    "packageManager": {
      "name": "pnpm",
      "version": "9.12.3"
    }
  }
}
```

Rules:

- Treat existing top-level `packageManager` as already set.
- Treat existing `devEngines.packageManager` as already set.
- Do not overwrite either declaration if present.
- Shallow-merge into existing `devEngines` when adding `packageManager`.
- Preserve existing `devEngines` entries.
- Keep the codemod name `add-package-manager`.
- Update the codemod description/logging to reflect `devEngines.packageManager`.

Example before:

```json
{
  "devEngines": {
    "runtime": { "name": "node", "version": "22.0.0" }
  }
}
```

Example after:

```json
{
  "devEngines": {
    "runtime": { "name": "node", "version": "22.0.0" },
    "packageManager": { "name": "pnpm", "version": "9.12.3" }
  }
}
```

## Workspace Conversion Tooling

Workspace conversion tooling should use `devEngines.packageManager` for package-manager metadata when adding or migrating declarations.

Rules:

- Add behavior should write `devEngines.packageManager` with shallow merge semantics.
- Remove behavior should remove top-level `packageManager` and/or `devEngines.packageManager` when those fields identify the manager being removed.
- Preserve either field if it identifies a different manager than the one being removed.
- When converting to a new manager, remove stale top-level `packageManager` first so implementation precedence does not keep resolving to the old manager.

## Documentation And Messaging

User-facing docs, errors, and generated comments should lead with `devEngines.packageManager` as the preferred declaration.

Guidance:

- Avoid saying only "add `packageManager`" in user-facing text.
- Prefer "add `devEngines.packageManager`".
- Mention top-level `packageManager` only as legacy/backward-compatible support where relevant.
- Update missing package manager errors to mention `devEngines.packageManager` as the recommended declaration.
- If the error still recommends the `add-package-manager` codemod, the codemod must write `devEngines.packageManager` first.
- Update `dangerouslyDisablePackageManagerCheck` comments/docs to mention both declaration fields or use declaration-neutral wording.

No `turbo.json` schema shape changes are needed beyond descriptive comments for existing options.

## Examples And Templates

Do not update examples, templates, or new project scaffolding to write `devEngines.packageManager` in the initial implementation.

Reason: examples/templates may be consumed by older stable Turborepo versions that do not yet support `devEngines.packageManager`.

Add a follow-up note: once the stable release containing detection support is available, examples/templates/new project scaffolding should be updated to prefer `devEngines.packageManager`.

## Testing Strategy

Add complete coverage across shared detection layers and representative end-to-end command paths.

Rust unit coverage:

- `PackageJson` round-trips `devEngines` and ignored fields.
- Valid `devEngines.packageManager` resolves to each supported manager.
- `pnpm` exact versions map through existing pnpm version logic.
- `yarn` exact versions map through existing Yarn classic/Berry logic.
- Top-level `packageManager` takes precedence over `devEngines.packageManager`.
- Missing top-level `packageManager` uses valid `devEngines.packageManager`.
- No declarations produces the missing package manager error and recommends `devEngines.packageManager`.
- Arrays are rejected.
- `null` is rejected.
- Non-object `devEngines` is rejected when top-level `packageManager` is absent.
- Non-object `devEngines.packageManager` is rejected.
- Missing `name` / missing `version` are rejected.
- Empty object produces combined shape error.
- Unsupported name is rejected before version validation.
- Non-string `name` / `version` are rejected.
- Empty strings are rejected.
- Semver ranges are rejected.
- URL versions are rejected.
- Invalid semver is rejected.
- Valid prerelease/build semver is accepted.
- Lockfile mismatch errors.
- Multiple lockfile behavior matches existing implicit detection.
- `dangerouslyDisablePackageManagerCheck` bypasses `devEngines.packageManager` parsing/mismatch and uses existing implicit detection.

Rust integration coverage:

- `turbo run` works when only `devEngines.packageManager` is present.
- Package graph construction works when only `devEngines.packageManager` is present.
- `turbo generate` uses the correct package-manager command when only `devEngines.packageManager` is present.
- `turbo prune` preserves `devEngines.packageManager` in the pruned root `package.json`.
- File/package watcher behavior reloads package manager state through existing root `package.json` watch behavior.
- Binding tests for `packages/turbo-repository/rust` are updated if they assert package manager detection behavior.

TypeScript coverage:

- `@turbo/workspaces` detects `devEngines.packageManager` with the same rules as Rust.
- `@turbo/workspaces` preserves top-level precedence.
- Detection errors lead with `devEngines.packageManager` in user-facing copy.
- Codemod writes `devEngines.packageManager`.
- Codemod shallow-merges existing `devEngines`.
- Codemod is idempotent when top-level `packageManager` exists.
- Codemod is idempotent when `devEngines.packageManager` exists.
- Workspace conversion add/remove behavior handles both declaration fields correctly.

New test fixtures should prefer `devEngines.packageManager` unless the test specifically covers legacy top-level `packageManager` behavior.

## Security Considerations

Parsing `devEngines.packageManager` must be pure and local.

Do not:

- Execute package manager binaries.
- Invoke Corepack.
- Access the network.
- Resolve URL versions.

Rejecting URL versions and non-semver integrity strings avoids turning package-manager detection into a command execution or network-resolution surface.

## Performance And Scalability

This feature should only read root metadata and existing root lockfile signals.

Expected overhead:

- Parse additional structured fields from root `package.json`.
- Reuse existing implicit detection for mismatch validation.
- Potentially read `yarn.lock` contents when existing Yarn detection would already do so.

Do not add:

- Workspace traversal.
- Per-package scanning.
- Dependency graph work.
- Additional package manager command execution.

Large repos should not see package-count-dependent overhead from this feature.

## Observability

No telemetry should be added.

Optional low-volume debug tracing is acceptable for local diagnosis, such as:

- Using `devEngines.packageManager` as the declaration source.
- Ignoring `devEngines.packageManager` because `dangerouslyDisablePackageManagerCheck` is enabled.
- Detecting a lockfile mismatch.

Avoid logging raw package.json values beyond manager names and safe file paths.

## Migration And Compatibility

No persisted data migration is required.

No cache migration is required.

No lockfile migration is required.

Resolved package-manager behavior should be identical for equivalent declarations after mapping to the internal `PackageManager` enum.

Top-level `packageManager` remains supported and authoritative for backward compatibility.

Examples/templates/new project scaffolding should be deferred until the stable release supports `devEngines.packageManager`.

## Non-Goals

This feature does not include:

- Full npm `devEngines` enforcement.
- Enforcing runtime package-manager version at command execution time.
- Supporting `devEngines.packageManager` arrays.
- Supporting semver ranges.
- Supporting missing `version`.
- Supporting package-manager aliases such as `berry`, `pnpm9`, or `yarn@berry`.
- Supporting URL versions for `devEngines.packageManager.version`.
- Interpreting or enforcing `onFail`.
- Adding telemetry.
- Changing cache keys intentionally.
- Adding persisted state.
- Updating examples/templates before stable support exists.
- Changing `ARCHITECTURE.md` or `CONTRIBUTING.md` unless later implementation changes touch their documented areas.

## Acceptance Criteria

- Root `devEngines.packageManager` is supported when top-level `packageManager` is absent.
- Top-level `packageManager` remains authoritative when present.
- Malformed `devEngines.packageManager` hard-errors when it is the active declaration source.
- `dangerouslyDisablePackageManagerCheck` bypasses `devEngines.packageManager` validation and uses existing implicit detection.
- Rust and TypeScript package-manager detection follow the same rules.
- Codemod writes `devEngines.packageManager` and preserves existing metadata.
- Workspace conversion tooling handles stale top-level declarations safely.
- Docs/errors lead with `devEngines.packageManager` as preferred.
- Examples/templates are explicitly deferred until stable support exists.
- Tests cover valid paths, malformed declarations, precedence, mismatch behavior, dangerous-disable behavior, codemod behavior, and representative CLI command paths.
