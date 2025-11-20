# Release Documentation

## Quick Start

### Release Turborepo CLI

1. Create a release by triggering the [Turborepo Release][1] workflow

   - Specify the semver increment using the SemVer Increment field (start with `prerelease`)
   - Check the "Dry Run" box to run the full release workflow without publishing any packages. Artifacts will be created that you can test with locally.

2. A PR is automatically opened to merge the release branch created in step 1 back into `main`

   - ⚠️ Merge this in! You don't need to wait for tests to pass (because they won't pass until after this PR is merged in). It's important to merge this branch soon after the publish is successful.

### Release `@turbo/repository`

1. Run [`bump-version.sh`][4] to update the versions of the packages. Merge in the changes to `main`.

2. Create a release by triggering the [Turborepo Library Release][5] workflow.
   - Check the "Dry Run" box to run the full release workflow without publishing any packages.

### Notes

- GitHub Release Notes are published automatically using the config from [`turborepo-release.yml`][2],
  triggered by the [`turbo-orchestrator`][3] bot.

---

## Turborepo CLI Release Process - In-Depth Guide

This section provides comprehensive documentation on how the Turborepo CLI is released, including the architecture, workflows, and detailed step-by-step processes.

### Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Version Management](#version-management)
3. [Release Workflow Stages](#release-workflow-stages)
4. [Packages Released](#packages-released)
5. [Platform-Specific Binaries](#platform-specific-binaries)
6. [Technical Reference](#technical-reference)
7. [Best Practices](#best-practices)

---

### Architecture Overview

The Turborepo release process is a multi-stage pipeline that:

1. **Manages versions centrally** via `version.txt` at the repository root
2. **Builds Rust binaries** for 6 different platforms (macOS, Linux, Windows on x64 and ARM64)
3. **Packages native binaries** as separate npm packages (e.g., `turbo-darwin-64`, `turbo-linux-arm64`)
4. **Publishes JavaScript packages** (main `turbo` package, `create-turbo`, codemods, ESLint plugins, etc.)
5. **Creates a release branch** with version bumps and automatically opens a PR to merge back to `main`

The entire process is orchestrated through a GitHub Actions workflow located at `.github/workflows/turborepo-release.yml`.

---

### Version Management

#### Version Storage

The single source of truth for the Turborepo version is **`version.txt`** at the repository root. This file contains two lines:

- Line 1: Version number (e.g., 2.6.1)
- Line 2: NPM dist-tag (e.g., latest, canary)

See: `version.txt`

#### Version Calculation

When a release is triggered, the `scripts/version.js` script:

1. Reads the current version from `version.txt`
2. Applies the specified SemVer increment (using the `semver` npm package)
3. Determines the npm dist-tag based on whether it's a pre-release
4. Writes the new version and tag back to `version.txt`

**Increment Types:**

| Increment Type | Description                       | Example                             | NPM Tag  |
| -------------- | --------------------------------- | ----------------------------------- | -------- |
| `prerelease`   | Bump canary of existing version   | `2.6.1-canary.0` → `2.6.1-canary.1` | `canary` |
| `prepatch`     | Create first canary of next patch | `2.6.1` → `2.6.2-canary.0`          | `canary` |
| `preminor`     | Create first canary of next minor | `2.6.1` → `2.7.0-canary.0`          | `canary` |
| `premajor`     | Create first canary of next major | `2.6.1` → `3.0.0-canary.0`          | `canary` |
| `patch`        | Stable patch release              | `2.6.1` → `2.6.2`                   | `latest` |
| `minor`        | Stable minor release              | `2.6.1` → `2.7.0`                   | `latest` |
| `major`        | Stable major release              | `2.6.1` → `3.0.0`                   | `latest` |

**Note**: Pre-release versions always use `canary` as the identifier unless overridden with the `tag-override` input.

#### Version Synchronization

Once the version is calculated, the `cli/Makefile` (target: `stage-release`) updates all package.json files by running `pnpm version` for each package to match `TURBO_VERSION`.

Additionally, the `packages/turbo/bump-version.js` postversion hook updates the `optionalDependencies` in `packages/turbo/package.json` to reference the correct versions of platform-specific packages.

See: `cli/Makefile` (stage-release target) and `packages/turbo/bump-version.js`

---

### Release Workflow Stages

The release workflow consists of 6 sequential and parallel stages:

```
┌─────────────────────────────────────────────────────────────┐
│ Stage 1: Version & Stage Commit                             │
│ - Calculate new version                                      │
│ - Create staging branch (staging-X.Y.Z)                      │
│ - Update all package.json files                              │
│ - Commit and tag (vX.Y.Z)                                     │
│ - Push staging branch                                         │
└──────────────────────┬──────────────────────────────────────┘
                       │
          ┌────────────┴────────────┐
          │                         │
          ▼                         ▼
┌──────────────────────┐  ┌──────────────────────┐
│ Stage 2:             │  │ Stage 3:             │
│ Rust Smoke Test      │  │ JS Smoke Test        │
│ - cargo groups test  │  │ - turbo run test     │
└──────────┬───────────┘  └──────────┬───────────┘
           │                         │
           └────────────┬────────────┘
                        │
                        ▼
           ┌────────────────────────┐
           │ Stage 4: Build Rust    │
           │ (5 parallel targets)   │
           │ - macOS x64 & ARM64    │
           │ - Linux x64 & ARM64    │
           │ - Windows x64          │
           └───────────┬────────────┘
                       │
                       ▼
           ┌────────────────────────┐
           │ Stage 5: NPM Publish   │
           │ - Pack native packages │
           │ - Publish native pkgs  │
           │ - Publish JS packages  │
           └───────────┬────────────┘
                       │
                       ▼
           ┌────────────────────────┐
           │ Stage 6: Release PR    │
           │ - Create PR to main    │
           └────────────────────────┘
```

#### Stage 1: Version & Stage Commit

**Job**: `stage` (runs on `ubuntu-latest`)

**Steps**:

1. Checkout repository at the specified SHA (defaults to latest commit on `main`)
2. Setup Node.js environment
3. Configure git with Turbobot credentials
4. Run `node scripts/version.js <increment>` to calculate the new version
5. Create staging branch: `staging-$(VERSION)` (e.g., `staging-2.6.2`)
6. Execute `make -C cli stage-release` which:
   - Verifies no unpushed commits exist
   - Verifies `version.txt` was updated
   - Updates all `package.json` files with the new version
   - Commits with message: `"publish $(VERSION) to registry"`
7. Create git tag: `v$(VERSION)` (e.g., `v2.6.2`)
8. Force push staging branch with tags to origin

**Output**: `stage-branch` (e.g., `staging-2.6.2`)

**Safety Checks**: The Makefile includes safety checks to verify no unpushed commits exist and that `version.txt` was properly updated before proceeding.

See: `cli/Makefile` (stage-release target)

#### Stage 2: Rust Smoke Test

**Job**: `rust-smoke-test` (depends on `stage`)

**Steps**:

1. Checkout the staging branch
2. Setup Turborepo environment (Rust toolchain only, skips Node.js setup)
3. Install `cargo-groups` tool (v0.1.9) for running grouped tests
4. Run: `cargo groups test turborepo`

This runs all Rust unit tests for the turborepo crates to ensure the code builds and tests pass before publishing.

#### Stage 3: JavaScript Package Tests

**Job**: `js-smoke-test` (depends on `stage`)

**Steps**:

1. Checkout the staging branch
2. Setup Node.js and install project dependencies
3. Install global Turbo from npm (using `ci-tag-override` if provided)
4. Run: `turbo run check-types test --filter="./packages/*" --color`

This runs TypeScript type checking and all Jest/Vitest tests for JavaScript packages.

**Note**: The `ci-tag-override` parameter is useful when a recent release was faulty and you need to test against a specific npm tag.

#### Stage 4: Build Rust Binaries

**Job**: `build-rust` (parallel matrix across 5 target platforms)

**Build Targets**:

| Platform    | Target Triple                | Runner                 | Binary Name |
| ----------- | ---------------------------- | ---------------------- | ----------- |
| macOS x64   | `x86_64-apple-darwin`        | `macos-13`             | `turbo`     |
| macOS ARM64 | `aarch64-apple-darwin`       | `macos-latest` (ARM64) | `turbo`     |
| Linux x64   | `x86_64-unknown-linux-musl`  | `ubuntu-latest`        | `turbo`     |
| Linux ARM64 | `aarch64-unknown-linux-musl` | `ubuntu-latest`        | `turbo`     |
| Windows x64 | `x86_64-pc-windows-msvc`     | `windows-latest`       | `turbo.exe` |

**Note**: Windows ARM64 (`aarch64-pc-windows-msvc`) is not currently built but the wrapper supports it for future compatibility.

**Build Configuration**:

The Rust binaries are built using the `release-turborepo` profile (inherits from release profile with stripping enabled) and Link-time optimization (LTO) enabled via the `CARGO_PROFILE_RELEASE_LTO=true` environment variable.

See: `Cargo.toml` (release-turborepo profile) and `.github/workflows/turborepo-release.yml`

**Build Steps**:

1. Install system dependencies (clang, musl-tools for Linux cross-compilation)
2. Install Protoc (v26.x) and Cap'n Proto (code generation tools)
3. Run: `cargo build --profile release-turborepo -p turbo --target <target>`
4. Upload binary artifact from `target/<target>/release-turborepo/turbo*`

#### Stage 5: NPM Publish

**Job**: `npm-publish` (depends on all previous stages)

This is the most complex stage with multiple sub-steps:

##### 5a. Prepare Rust Artifacts

1. Download all platform-specific binary artifacts from Stage 4
2. Move binaries to platform-specific directories:
   ```
   rust-artifacts/turbo-aarch64-apple-darwin    → cli/dist-darwin-arm64/turbo
   rust-artifacts/turbo-x86_64-apple-darwin     → cli/dist-darwin-x64/turbo
   rust-artifacts/turbo-aarch64-unknown-linux-musl → cli/dist-linux-arm64/turbo
   rust-artifacts/turbo-x86_64-unknown-linux-musl  → cli/dist-linux-x64/turbo
   rust-artifacts/turbo-x86_64-pc-windows-msvc  → cli/dist-windows-x64/turbo.exe
   ```

##### 5b. Build JavaScript Packages

Execute `make -C cli build` which runs `turbo build copy-schema` with filters for all JavaScript/TypeScript packages. This builds all TypeScript packages and copies the JSON schema to the appropriate locations.

See: `cli/Makefile` (build target)

##### 5c. Pack and Publish Native Packages

Execute `turbo release-native` which invokes the `@turbo/releaser` tool.

**The `@turbo/releaser` tool** (`packages/turbo-releaser/`):

1. **Reads version and tag** from `version.txt`
2. **For each platform** (6 total):
   - Generates a native package structure with platform-specific metadata (name, os, cpu, etc.)
   - Copies `LICENSE` and `README.md` from template
   - For Windows: includes a `bin/turbo` Node.js wrapper script (to work around npm `.exe` stripping)
   - Copies the prebuilt binary from `cli/dist-<os>-<arch>/`
   - Makes the binary executable (`chmod +x` on Unix)
   - Creates a `.tar.gz` archive
   - Publishes to npm: `npm publish turbo-<os>-<arch>.tar.gz --tag <npm-tag>`

See: `packages/turbo-releaser/` for native package generation logic

**Published native packages**:

- `turbo-darwin-64`
- `turbo-darwin-arm64`
- `turbo-linux-64`
- `turbo-linux-arm64`
- `turbo-windows-64`
- `turbo-windows-arm64` (package structure only, binary not yet built)

##### 5d. Pack and Publish JavaScript Packages

Execute `make -C cli publish-turbo` which:

1. **Packs all packages** to tarballs using `pnpm pack`
2. **Publishes in fixed order** to npm with the appropriate dist-tag (if not `--skip-publish`)

See: `cli/Makefile` (publish-turbo target)

**Why fixed order?**

- Prevents race conditions where dependent packages are published before their dependencies
- Ensures `turbo` is published last so the platform specific binaries that it depends on are available.

**Dry Run**: If the workflow was triggered with `dry_run: true` or the Makefile is called with `--skip-publish`, the publish commands are skipped, allowing you to test the entire pipeline without publishing.

#### Stage 6: Merge Release PR

A release PR is automatically generated. Merge it as soon as possible after publishing has completed.

---

### Packages Released

The Turborepo release publishes **15 npm packages** (6 native + 9 JavaScript):

#### Native Packages (Platform-Specific Binaries)

| Package               | Description                | OS       | Arch    |
| --------------------- | -------------------------- | -------- | ------- |
| `turbo-darwin-64`     | macOS Intel binary         | `darwin` | `x64`   |
| `turbo-darwin-arm64`  | macOS Apple Silicon binary | `darwin` | `arm64` |
| `turbo-linux-64`      | Linux x64 binary (musl)    | `linux`  | `x64`   |
| `turbo-linux-arm64`   | Linux ARM64 binary (musl)  | `linux`  | `arm64` |
| `turbo-windows-64`    | Windows x64 binary         | `win32`  | `x64`   |
| `turbo-windows-arm64` | Windows ARM64 binary       | `win32`  | `arm64` |

**Note**: Native packages use musl for Linux to ensure maximum compatibility across distributions.

#### JavaScript/TypeScript Packages

| Package               | Description                                               | Location                        |
| --------------------- | --------------------------------------------------------- | ------------------------------- |
| **`turbo`**           | Main CLI package (platform detection and loader)          | `packages/turbo/`               |
| `create-turbo`        | Scaffold new Turborepo projects                           | `packages/create-turbo/`        |
| `@turbo/codemod`      | Codemods for version upgrades                             | `packages/turbo-codemod/`       |
| `turbo-ignore`        | CI/CD ignore utility (determines if deployment is needed) | `packages/turbo-ignore/`        |
| `@turbo/workspaces`   | Workspace management tools                                | `packages/turbo-workspaces/`    |
| `@turbo/gen`          | Generator for extending Turborepo                         | `packages/turbo-gen/`           |
| `eslint-plugin-turbo` | ESLint plugin for Turborepo                               | `packages/eslint-plugin-turbo/` |
| `eslint-config-turbo` | Shared ESLint configuration                               | `packages/eslint-config-turbo/` |
| `@turbo/types`        | TypeScript types and JSON schema                          | `packages/turbo-types/`         |

#### Main Package: `turbo`

The `turbo` package is unique:

1. **Doesn't contain the binary** - it's a JavaScript wrapper that:

   - Detects the current platform and architecture
   - Requires the appropriate platform-specific package
   - Falls back to x64 on ARM64 for macOS/Windows (Rosetta/emulation support)
   - Provides just-in-time installation if the platform package is missing

2. **Declares platform packages as optional dependencies** - all six platform-specific packages are listed as `optionalDependencies` in the package.json, allowing npm to install only the relevant one for the current platform.

3. **Entry point**: `packages/turbo/bin/turbo` (Node.js script)

See: `packages/turbo/package.json` and `packages/turbo/bin/turbo`

---

### Platform-Specific Binaries

#### Binary Selection Logic

When a user runs `turbo`, the `packages/turbo/bin/turbo` script:

1. **Checks `TURBO_BINARY_PATH`** environment variable (for local development)
2. **Detects platform**: `process.platform` and `process.arch`
3. **Maps to package name** using a platform-to-package mapping
4. **Attempts to require the correct platform package**
5. **Falls back** to x64 on ARM64 for macOS and Windows (Rosetta 2 / emulation support)
6. **Just-in-time install**: If the package is missing, attempts `npm install` for that specific platform
7. **Errors with diagnostics** if all attempts fail

See: `packages/turbo/bin/turbo` for the complete platform detection logic

#### Windows Special Handling

Windows has special considerations:

1. **Binary name**: `turbo.exe`
2. **npm `.exe` stripping issue**: npm strips `.exe` files from the `bin/` directory
3. **Solution**: Native Windows packages include a `bin/turbo` Node.js wrapper script that spawns `turbo.exe` and forwards all arguments and stdio

See: `packages/turbo-releaser/` for the Windows wrapper generation

---

### Technical Reference

#### Key Scripts and Commands

| Script/Command                                     | Location                   | Purpose                                        |
| -------------------------------------------------- | -------------------------- | ---------------------------------------------- |
| `node scripts/version.js <increment>`              | `scripts/version.js`       | Calculate new version and update `version.txt` |
| `make -C cli stage-release`                        | `cli/Makefile`             | Update all package.json versions and commit    |
| `cargo build --profile release-turborepo -p turbo` | `Cargo.toml`               | Build Rust binary for release                  |
| `turbo release-native`                             | `cli/turbo.json`           | Pack and publish native packages               |
| `make -C cli build`                                | `cli/Makefile`             | Build all JavaScript packages                  |
| `make -C cli publish-turbo`                        | `cli/Makefile`             | Pack and publish all packages                  |
| `pnpm version <version> --allow-same-version`      | package.json               | Update package version                         |
| `turboreleaser --version-path ../version.txt`      | `packages/turbo-releaser/` | Pack native packages                           |

#### Environment Variables

| Variable                    | Purpose                                        | Example              |
| --------------------------- | ---------------------------------------------- | -------------------- |
| `TURBO_VERSION`             | Version to release (read from version.txt)     | `2.6.2`              |
| `TURBO_TAG`                 | npm dist-tag (read from version.txt)           | `latest` or `canary` |
| `NPM_TOKEN`                 | npm authentication token (from GitHub Secrets) | `npm_xxx...`         |
| `CARGO_PROFILE_RELEASE_LTO` | Enable link-time optimization for Rust         | `true`               |
| `TURBO_BINARY_PATH`         | Override binary path (development only)        | `/path/to/turbo`     |

#### Rust Build Profile

The `release-turborepo` profile inherits from the release profile with debug symbol stripping enabled. Link-time optimization is enabled via the `CARGO_PROFILE_RELEASE_LTO=true` environment variable during the build.

See: `Cargo.toml` (release-turborepo profile)

#### Workflow Inputs Reference

| Input          | Type    | Required | Default      | Description                                                                                        |
| -------------- | ------- | -------- | ------------ | -------------------------------------------------------------------------------------------------- |
| `increment`    | choice  | Yes      | `prerelease` | SemVer increment type: `prerelease`, `prepatch`, `preminor`, `premajor`, `patch`, `minor`, `major` |
| `dry_run`      | boolean | No       | `false`      | Skip npm publish and PR creation (test mode)                                                       |
| `tag-override` | string  | No       | -            | Override npm dist-tag (e.g., for backports)                                                        |

#### Common npm Dist-tags

| Tag        | Usage                              | Example Version    |
| ---------- | ---------------------------------- | ------------------ |
| `latest`   | Stable releases                    | `2.6.2`            |
| `canary`   | Pre-release versions               | `2.6.3-canary.0`   |
| `next`     | Beta releases (manual override)    | `3.0.0-beta.1`     |
| `backport` | Backported fixes (manual override) | `2.5.2-backport.0` |

Users can install specific tags:

```bash
npm install turbo@latest    # Stable
npm install turbo@canary    # Pre-release
npm install turbo@2.6.2     # Specific version
```

#### Rust Crate Versions

**Important Note**: Rust crate versions in `Cargo.toml` are **not updated during releases**. The Rust crates remain at version `0.1.0` in the manifest.

The version management is handled entirely through:

- `version.txt` for the release pipeline
- npm package versions for distribution

This is because the Rust binary is never published to crates.io; it's only published to npm as platform-specific packages.

---

### Best Practices

1. **Always start with canary releases**: When releasing new features, start with `prerelease` to publish a canary version. Test it in production before promoting to stable.

2. **Use dry run for testing**: When in doubt, use `dry_run: true` to test the entire pipeline without publishing.

3. **Monitor the release PR**: After a successful release, merge the release PR promptly. Don't let it sit for days as it can cause conflicts.

4. **Check npm after publishing**: Verify that all packages were published correctly:

   ```bash
   npm view turbo@<version>
   npm view turbo-darwin-64@<version>
   npm view create-turbo@<version>
   # ... etc
   ```

5. **Handle failed releases carefully**: If a release fails mid-publish (some packages published, others not), document which packages were published and manually publish the rest if needed.

6. **Backporting**: Use `tag-override` when backporting fixes to older major versions. For example, releasing `2.5.3` when `main` is on `3.0.0`.

---

[1]: https://github.com/vercel/turborepo/actions/workflows/turborepo-release.yml
[2]: https://github.com/vercel/turborepo/blob/main/.github/turborepo-release.yml
[3]: https://github.com/apps/turbo-orchestrator
[4]: https://github.com/vercel/turborepo/blob/main/packages/turbo-repository/scripts/bump-version.sh
[5]: https://github.com/vercel/turborepo/actions/workflows/turborepo-library-release.yml
