# devEngines.packageManager PR Plan

This plan breaks the `devEngines.packageManager` work from `SPEC.md` into reviewable PRs.

## PR 1: Rust support for `devEngines.packageManager`

- Add structured `PackageJson.dev_engines` support.
- Preserve unknown and ignored `devEngines` fields during serialization.
- Implement Rust detection precedence: top-level `packageManager`, then `devEngines.packageManager`, then missing-declaration error.
- Validate supported names, exact semver versions, malformed objects, whitespace, and empty values.
- Reuse existing pnpm and Yarn version mapping.
- Add lockfile mismatch validation.
- Wire `dangerouslyDisablePackageManagerCheck` to bypass declaration validation and use existing implicit detection behavior.
- Update Rust diagnostics and messaging to recommend `devEngines.packageManager`.
- Add Rust unit coverage and representative CLI integration coverage.

## PR 2: TypeScript detection parity

- Update `@turbo/workspaces` package-manager detection to support `devEngines.packageManager`.
- Match Rust precedence, validation, exact-semver-only behavior, array rejection, malformed-object handling, and ignored unknown properties.
- Update TypeScript user-facing errors to prefer `devEngines.packageManager`.
- Add TypeScript detection tests.

## PR 3: Codemod update

- Keep the `add-package-manager` transformer name.
- Write `devEngines.packageManager` instead of top-level `packageManager`.
- Treat existing top-level `packageManager` as already set.
- Treat existing `devEngines.packageManager` as already set.
- Shallow-merge into existing `devEngines`.
- Update codemod description and logging.
- Add tests for output shape, merge behavior, and idempotency for both declaration fields.

## PR 4: Workspace conversion tooling

- Update add, remove, and convert behavior to handle both declaration fields.
- Remove stale top-level `packageManager` when converting to a new manager so precedence cannot keep resolving to the old manager.
- Preserve either declaration field when it identifies a different manager than the one being removed.
- Add tests for add, remove, convert, stale top-level removal, and preservation behavior.

## PR 5: Docs and user-facing copy

- Update docs, errors, comments, and generated schema descriptions to prefer `devEngines.packageManager`.
- Mention top-level `packageManager` only as legacy or backward-compatible support where useful.
- Update `dangerouslyDisablePackageManagerCheck` descriptions to mention both declaration fields or use declaration-neutral wording.

## PR 6: Examples, templates, and scaffolding

- Update examples, templates, and new project scaffolding to write `devEngines.packageManager`.
- This PR should happen after a stable release includes `devEngines.packageManager` detection support, so generated projects remain compatible with currently supported Turborepo versions.
