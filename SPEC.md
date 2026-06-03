# TypeScript Project References Syncer

## Goal

Create `@turbo/typescript`, a publishable package that helps large Turborepo monorepos incrementally adopt TypeScript Project References with an opinionated paved path.

The tool owns keeping TypeScript project-reference configuration synchronized with the workspace package graph. It should make safe edits automatically, keep diffs small and deterministic, and report blockers clearly when a package cannot yet participate.

## Non-Goals

- Do not run `tsc`, `tsc -b`, or validate compiler output.
- Do not create package-level `tsconfig.json` files.
- Do not support filtered or partial workspace operation.
- Do not support watch mode.
- Do not support arbitrary non-package project references or app-internal subprojects in v1.
- Do not parse lockfiles at runtime.
- Do not mutate files outside the workspace.
- Do not install dependencies or invoke package-manager lifecycle scripts.

## Package

- Package name: `@turbo/typescript`.
- Published to npm.
- Provide both ESM and CommonJS-compatible entrypoints.
- Provide a CLI binary.
- Expose a small experimental programmatic API used by the CLI.
- May use `@turbo/utils` as an internal workspace dependency during development, but `@turbo/utils` must be bundled into the published artifact and must not be a runtime dependency for consumers.
- Bundle only the needed utility code to avoid pulling unrelated behavior or dependencies.

Runtime dependencies:

- `fast-glob`
- `js-yaml`
- `jsonc-parser`
- `semver`

Peer/dev dependency:

- `typescript` is a peer dependency and dev dependency. The CLI must resolve TypeScript from the target workspace root, not from the CLI package location. If unavailable, fail with install guidance.

## CLI

Command namespace:

```sh
turbo-typescript project-references init
turbo-typescript project-references check
turbo-typescript project-references write
turbo-typescript project-references candidates
```

Global command behavior:

- Exit codes are only `0` and `1`.
- Human-readable output is default.
- `--json` is available on all commands and is a documented stable API with `version: 1` in the payload.
- JSON output uses workspace-relative paths, not absolute paths, except unavoidable low-level unexpected errors.
- `--verbose` is available for detailed graph/config decisions.
- Colors are allowed only when appropriate for the terminal and must not be the only signal.
- No spinners.

### `init`

Initializes the feature.

Behavior:

- Requires an existing root `turbo.json` or `turbo.jsonc`.
- Fails if `typescriptProjectReferences` already exists, unless `--force` is passed.
- Detects packages already participating by reading root `tsconfig.json` `references`, resolving each reference to a workspace package root.
- Writes `typescriptProjectReferences` to the root Turbo config.
- Adds packages without package-root `tsconfig.json` to `ignored`.
- Adds every package not already referenced by root `tsconfig.json` and not ignored to `excluded`.
- Normalizes root/package TypeScript configs for the packages that were already referenced by root `tsconfig.json`.
- Does not immediately promote newly initialized exclusions. Users can run `write` after `init` to perform the first migration step.
- `--force` recomputes config from current root references, while preserving existing `ignored` entries.
- `--dry-run` reports intended changes without writing.

### `check`

Read-only CI command.

Behavior:

- Fails if `typescriptProjectReferences` is absent.
- Fails if root `tsconfig.json` is missing.
- Computes the same desired state as `write`.
- Fails if root Turbo config normalization, root `tsconfig.json`, or owned package `tsconfig.json` fields differ semantically from the desired state.
- Does not fail on unrelated JSON formatting.
- Never modifies files.
- Reports `run turbo-typescript project-references write` when changes are needed.

### `write`

Converges the repo to the maximum valid Project References state.

Behavior:

- Fails if `typescriptProjectReferences` is absent.
- Creates root `tsconfig.json` if missing.
- Never creates package `tsconfig.json` files.
- Computes the maximum valid set of packages that can participate without invalid references.
- Updates root Turbo config to match current migration state.
- Removes packages from `excluded` when they are now valid.
- Adds packages to `excluded` when they cannot be included without invalidity.
- Adds packages without `tsconfig.json` to `ignored`.
- Preserves existing `ignored` entries. `ignored` is sticky until the user edits it.
- Removes stale paths from `excluded` and `ignored` when the package no longer exists.
- Updates root `tsconfig.json` and valid package `tsconfig.json` files.
- Leaves excluded and ignored package `tsconfig.json` files untouched.
- `--dry-run` reports intended changes without writing.

Writes are planned transactionally:

- Read and parse every required file first.
- Compute all intended edits before writing any file.
- If any required parse/validation/read step fails, write nothing.
- After all edits are computed, write changed files directly. Do not use temp-file atomic renames, to avoid stale temp files.
- If a filesystem error occurs during writing, report exactly what was written and what failed.

### `candidates`

Read-only migration guidance.

Behavior:

- Fails if `typescriptProjectReferences` is absent.
- Does not mutate files.
- Recomputes the desired graph/migration state.
- Lists excluded packages that can now be removed from `excluded` and included safely.
- Lists newly discovered packages separately as `New packages`.
- Reports blockers, including excluded dependencies and cycles.
- Exits `0` even when no candidates exist.
- Default output should say to run `write` to migrate candidates.
- `--verbose` may include before/after excluded-list details and candidate sort data.

## Config

Config lives in the root `turbo.json` or `turbo.jsonc` under `typescriptProjectReferences`.

Presence of the key activates the feature.

Accepted input forms:

```jsonc
{
  "typescriptProjectReferences": true
}
```

```jsonc
{
  "typescriptProjectReferences": {}
}
```

```jsonc
{
  "typescriptProjectReferences": {
    "excluded": ["packages/not-ready"],
    "ignored": ["tooling/config-only"]
  }
}
```

Semantics:

- `excluded`: packages that are TypeScript-capable but cannot currently participate. The tool may remove entries when they become valid and may add entries when needed to preserve a valid graph.
- `ignored`: packages outside Project References enforcement. Dependencies on ignored packages do not block inclusion and do not create references. `ignored` is sticky once present; `write` preserves it until the user removes it.
- No `enabled` list exists. Packages are enabled by default unless excluded or ignored.
- Empty object and `true` have equivalent semantics.
- Mutating commands normalize config.
- If both `excluded` and `ignored` are empty, normalize to `typescriptProjectReferences: true`.
- Empty arrays are omitted.
- Non-empty arrays are sorted alphabetically by workspace-relative package path.
- Duplicate entries are deduped by mutating commands. `check` fails if normalization would change the config.
- Invalid value types are hard errors.
- Unknown paths are errors in read-only validation, and stale paths are removed by mutating commands when expected.
- Entries are workspace-relative package paths only. Package names, absolute paths, leading `./`, trailing slashes, and paths containing `..` are normalized or rejected according to safety rules.
- Backslashes in config may be accepted on Windows but are written as POSIX paths.

Root-only behavior:

- Only the root Turbo config is read for `typescriptProjectReferences`.
- Package-level Turbo config usage is ignored by this tool. If existing Turbo schema validation can reject package-level placement without new architecture, it should do so; otherwise do not create new architecture just for this.
- If both root `turbo.json` and `turbo.jsonc` exist, fail.

Schema:

- Update the Turborepo JSON schema so editors understand `typescriptProjectReferences` where supported.
- Runtime validation remains stricter than schema validation.

## Workspace Discovery

The package should hand-roll strict JavaScript workspace discovery, using or bundling existing utility code where helpful.

Supported workspace config formats:

- `pnpm-workspace.yaml` with `packages`.
- Root `package.json` with `workspaces: string[]`.
- Root `package.json` with `workspaces.packages`.
- npm, Yarn, pnpm, and Bun workspaces through those JavaScript package-manager formats.

Unsupported in v1:

- Lerna config.
- Rush config.
- Lage config.
- Nx project config.
- Arbitrary package-manager extensions beyond the JS package-manager workspace formats above.

Discovery rules:

- Require a Turborepo workspace root with root `turbo.json` or `turbo.jsonc`.
- Non-multi-package workspaces are not supported.
- The root package itself is not a project-reference participant.
- Private packages are included.
- Workspace glob negations are honored.
- Existing behavior takes precedence when both `pnpm-workspace.yaml` and root `package.json#workspaces` exist; match current Turborepo utility behavior.
- Unsafe workspace globs that escape the root must fail rather than be ignored.
- Resolve realpaths and reject package roots outside the workspace.
- Do not follow symlinked package roots outside the workspace.
- Malformed package manifests are hard errors.
- Duplicate workspace package names are hard errors.
- Workspace packages without `name` are hard errors.

## Package Manager Semantics

Package-manager detection should be reimplemented in JavaScript to match existing Turborepo behavior.

Rules:

- Follow existing Turborepo detection policy for `packageManager` and lockfile fallback.
- If multiple signals conflict in a way existing Turborepo behavior treats as invalid, fail.
- Do not parse lockfiles at runtime.
- Do not invoke installs at runtime.

Internal edge detection must match what package managers really link as local workspace packages.

Requirements:

- If the package manager would create a local workspace link, the dependency is an internal edge.
- If the package manager would pull from the npm registry, the dependency is not an internal edge.
- Implement and test npm, pnpm, Yarn, and Bun behavior empirically.
- Use fixture/integration tests that run the package managers to verify workspace-linking semantics, then encode those rules in the syncer.

## Graph Construction

Build a direct dependency graph from discovered workspace manifests.

Rules:

- Graph nodes are workspace packages, excluding the root package.
- Dependency keys are matched to workspace package `name` fields according to package-manager workspace-linking semantics.
- Include only `dependencies` and `devDependencies` edges.
- Exclude `optionalDependencies` and `peerDependencies` edges.
- If a dependency appears in multiple manifest sections, include it if it appears in `dependencies` or `devDependencies`; do not warn.
- Direct edges only. Do not add transitive dependencies as references.
- Declared package graph is the truth. Do not scan TypeScript imports.
- Dependencies on ignored packages are terminal and do not create references or block inclusion.

Cycle handling:

- Detect cycles in the included direct graph after removing ignored nodes.
- TypeScript Project References cycles are unsupported; therefore the syncer must not generate cyclic references.
- Packages in cycles and their dependents remain excluded until the cycle is broken or relevant packages are ignored.
- `candidates` and diagnostics should report cycles as blockers.

## Valid Set Computation

Effective Project References participants are the maximum valid set of packages satisfying:

- Package is a workspace package.
- Package is not ignored.
- Package has package-root `tsconfig.json`.
- Package is not part of a detected cycle.
- All included direct workspace dependencies are in the valid set or ignored.

Packages outside that set are excluded unless ignored.

Important behavior:

- Users start with many exclusions and migrate inward from leaves.
- `write` automatically removes exclusions when packages become valid.
- `write` automatically adds exclusions when needed to avoid invalid references.
- Root references include the maximum valid set, not all non-ignored packages.

## TypeScript Config Handling

Use TypeScript’s compiler API to parse and resolve config inheritance.

Rules:

- Support whatever TypeScript supports for JSONC syntax and `extends` resolution, subject to workspace boundary rules.
- Resolve TypeScript from the target workspace root using `createRequire(root/package.json)`.
- Use TypeScript config parsing to determine effective `compilerOptions.composite`.
- Never create a TypeScript program or run compiler validation.
- Do not load plugins or execute arbitrary code.
- Resolved `extends` files must be inside the workspace by realpath.
- If TypeScript resolves `extends` outside the workspace, fail.
- Package-name `extends` is allowed if TypeScript resolves it inside the workspace.

### Root `tsconfig.json`

Root `tsconfig.json` is the tsserver-discoverable solution config and is owned for solution-shape fields.

`write` must:

- Create root `tsconfig.json` if missing.
- Ensure `files: []`.
- Remove root `include`.
- Sync root `references` to the computed valid package set.
- Preserve unrelated fields such as `extends`, `compilerOptions`, `exclude`, comments, and unrelated formatting.

When creating a new root config, use:

```json
{
  "files": [],
  "references": []
}
```

### Package `tsconfig.json`

For valid participating packages, `write` must:

- Sync `references` to direct valid workspace dependencies only.
- Set `references` to an empty array when no direct valid dependencies exist.
- Ensure effective `compilerOptions.composite` is `true`.
- Add local `compilerOptions.composite: true` only when TypeScript effective config does not already provide `composite: true` through `extends`.
- Override inherited `composite: false` locally with `true`.
- Leave package `include`, `files`, and unrelated fields alone.

Excluded and ignored package configs are never touched.

Missing package `tsconfig.json`:

- Package is added to `ignored` by mutating commands.
- Package references to it are removed.
- The package config is not created.

## Reference Formatting and Preservation

Reference paths:

- Root references use workspace-relative package paths, e.g. `packages/ui`.
- Package references use POSIX relative paths from the package directory to dependency package directory, e.g. `../ui` or `../../packages/ui`.
- References point to package directories, not explicit `tsconfig.json` files.
- Paths are canonicalized with no trailing slash, no redundant segments, and POSIX separators.
- Reference arrays are sorted alphabetically by canonical `path`.

Reference object preservation:

- A valid reference is one whose resolved package path is currently expected.
- Existing valid reference objects preserve extra properties such as `prepend`.
- The `path` property is canonicalized even when preserving extra properties.
- Missing references are added as `{ "path": "..." }`.
- Invalid, stale, excluded, ignored, non-package, or internal-subproject reference objects are removed entirely.

## JSON/JSONC Editing

Use JSONC-preserving edits for owned changes.

Goals:

- Minimize diffs.
- Preserve comments and unrelated formatting where practical.
- Avoid whole-file formatting.
- Maintain deterministic ordering for arrays the tool owns.

Field placement guidance:

- Root `tsconfig.json`: when adding both, place `files` before `references`.
- Package `tsconfig.json`: add `compilerOptions.composite` inside existing `compilerOptions`; if creating `compilerOptions`, place it before `references` when practical.
- Root Turbo config: add `typescriptProjectReferences` without reordering unrelated top-level keys.

Comment behavior:

- Preserve unrelated comments best-effort.
- Replacing `typescriptProjectReferences` may lose comments inside/around the old block.
- Removing array entries may remove comments attached to those entries.

## Diagnostics and UX

Default output should be concise but complete. Do not cap blocker output.

Diagnostics should include:

- Packages added to or removed from `excluded`.
- Packages added to `ignored`.
- Stale config paths removed.
- Root/package config mismatches for `check`.
- Packages blocked by excluded dependencies.
- Packages blocked by cycles.
- Missing root `tsconfig.json` in `check`.
- Missing package `tsconfig.json` decisions in mutating commands.
- Unsupported external `extends` resolution.
- Malformed JSON/JSONC with line/column when possible.

Default `write` output should emphasize migration state over mechanics:

- Counts of root/package tsconfigs updated.
- Packages moved in/out of `excluded`.
- Packages added to `ignored`.
- Blockers and next actions.

`--verbose` should additionally include:

- Changed file paths.
- Package dependency blockers.
- Ignored/excluded decision details.
- Candidate sort data.

Accessibility:

- No color-only meaning.
- Plain, deterministic output in CI/non-TTY.
- No spinner-only loading state.

## Security

- Never mutate outside the workspace.
- Resolve realpaths when enforcing workspace boundaries.
- Reject package roots outside the workspace.
- Reject `extends` files resolved outside the workspace.
- Do not execute package-manager lifecycle scripts.
- Do not install dependencies.
- Do not create a TypeScript program or load TypeScript plugins.
- Do not collect or emit absolute paths in normal JSON output.

## Performance and Scalability

The tool targets large monorepos.

Requirements:

- Read files concurrently where safe.
- Avoid TypeScript program creation.
- Avoid import scanning.
- Avoid lockfile parsing at runtime.
- Use direct graph algorithms over package nodes and direct edges.
- Cache parsed manifests/configs within a single command invocation.
- Keep writes limited to files with semantic changes.
- Deterministic sorting for stable diffs.

## Telemetry

Follow existing Turborepo JavaScript package telemetry conventions.

If telemetry is present, collect only coarse data:

- Command name.
- Success/failure.
- Duration bucket.
- Package count.
- Excluded count.
- Ignored count.
- Candidate count.
- Error category.

Do not send:

- Workspace paths.
- Package names.
- Dependency names.
- Config contents.

## Testing Strategy

Primary strategy: fixture-based integration tests.

Each fixture should contain:

- Root Turbo config.
- Root/package manifests.
- Root/package TypeScript configs.
- Expected post-command file tree or structured result.

Coverage:

- `init`, `init --force`, `init --dry-run`.
- `check` success/failure.
- `write`, `write --dry-run`.
- `candidates`.
- `--json` output schemas.
- `--verbose` output basics.
- Root `turbo.json` and `turbo.jsonc`.
- Both root config files present.
- Missing root `tsconfig.json`.
- Missing package `tsconfig.json` auto-ignored.
- Sticky `ignored` behavior.
- Excluded convergence.
- Stale `excluded` and `ignored` removal.
- Root solution shape normalization.
- Package reference normalization.
- Preservation of reference metadata.
- `composite` inherited through `extends`.
- Local `composite` insertion when needed.
- External `extends` failure.
- Cycles.
- Dependency kind filtering.
- Duplicate package names.
- Nameless package failure.
- Malformed JSON/JSONC.
- Workspace glob negations.
- Unsafe workspace globs and symlink escape rejection.

Package-manager semantics:

- Include npm, pnpm, Yarn, and Bun fixtures.
- Run empirical install/linking tests in CI to verify which dependency specifiers become local workspace links.
- Encode those observed rules in runtime graph construction.
- Run all tests rather than gating package-manager semantic tests separately.

Snapshot guidance:

- Snapshot expected fixture file trees where useful.
- Avoid brittle snapshots for full human CLI output, except high-level golden examples.
- Assert JSON output structurally.

Unit tests:

- Path normalization.
- Graph algorithms and cycle detection.
- Config normalization.
- Reference preservation/canonicalization.

## Programmatic API

Expose small APIs for CLI reuse and external automation:

```ts
initProjectReferences(options)
checkProjectReferences(options)
writeProjectReferences(options)
getProjectReferenceCandidates(options)
```

Options should remain intentionally small:

- `cwd`
- `dryRun` for mutating commands
- output mode controls used by CLI wrappers as needed

Do not expose v1 options for graph overrides, tsconfig filenames, filters, or formatting knobs.

Return structured results compatible with `--json` output.
