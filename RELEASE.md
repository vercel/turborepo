# Release Documentation

## Quick Start

### Automated Canary Releases

Canary releases run on an hourly schedule via the [Release workflow][1]:

1. Runs every hour via cron, skipping if no relevant files (`crates/`, `packages/`, `cli/`) changed since the last canary tag
2. Skips if the latest commit is a release PR merge (to avoid releasing the version bump itself)
3. Publishes to npm with the `canary` tag
4. Opens a PR with auto-merge enabled to merge the version bump back to `main`

No manual intervention required for canary releases.

### Manual Releases (Stable/Custom)

1. Create a release by triggering the [Turborepo Release][1] workflow
   - For stable releases, use `patch`, `minor`, or `major`
   - For custom pre-releases, use `prepatch`, `preminor`, or `premajor`
   - Check the "Dry Run" box to test the workflow without publishing

2. A PR is automatically opened to merge the release branch back into `main`
   - Merge this promptly to avoid conflicts

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
2. [Automated Canary Releases](#automated-canary-releases)
3. [Version Management](#version-management)
4. [Release Workflow Stages](#release-workflow-stages)
5. [Packages Released](#packages-released)
6. [Platform-Specific Binaries](#platform-specific-binaries)
7. [Technical Reference](#technical-reference)
8. [Best Practices](#best-practices)

---

### Architecture Overview

The Turborepo release process is a multi-stage pipeline that:

1. **Manages versions centrally** via `version.txt` at the repository root
2. **Builds Rust binaries** for 6 different platforms (macOS, Linux, Windows on x64 and ARM64)
3. **Packages native binaries** as separate npm packages (e.g., `turbo-darwin-64`, `turbo-linux-arm64`)
4. **Publishes JavaScript packages** (main `turbo` package, `create-turbo`, codemods, ESLint plugins, etc.)
5. **Aliases versioned documentation** to subdomains (e.g., `v2-5-4.turborepo.dev`)
6. **Creates a release branch** with version bumps and automatically opens a PR to merge back to `main`

The process is orchestrated through one GitHub Actions workflow:

- **`.github/workflows/turborepo-release.yml`** - Handles both scheduled canary releases and manual releases

---

### Automated Canary Releases

The canary release system runs on an hourly cron schedule, publishing a new canary version if relevant files have changed since the last release.

#### How It Works

```
┌─────────────────────────────────────────────────────────────┐
│ Hourly cron fires                                            │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│ check-skip job                                               │
│ - Skips if no relevant files changed since last release      │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│ turborepo-release.yml continues                              │
│ - Stages version bump                                        │
│ - Runs smoke tests                                           │
│ - Builds binaries                                            │
│ - Publishes to npm                                           │
│ - Aliases versioned docs                                     │
│ - Creates PR with auto-merge                                 │
└─────────────────────────────────────────────────────────────┘
```

#### Skip Detection

The `check-skip` job finds the commit that last modified `version.txt` (which is always the release PR merge) and diffs from there to HEAD. If no files in `crates/`, `packages/`, or `cli/` changed since that commit, there's nothing new to release and the run is skipped.

#### Concurrency

All releases (scheduled and manual) share a single concurrency group:

```yaml
concurrency:
  group: turborepo-release
  cancel-in-progress: false
```

This ensures only one release runs at a time. If a manual release is triggered while a scheduled run is in progress, it waits for the current run to finish.

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

The release workflow consists of 7 sequential and parallel stages:

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
          ┌────────────┴────────────┐
          │                         │
          ▼                         ▼
┌──────────────────────┐  ┌──────────────────────┐
│ Stage 6:             │  │ Stage 7:             │
│ Alias Versioned Docs │  │ Release PR           │
│ - Find deployment    │  │ - Create PR to main  │
│ - Create subdomain   │  │ - Include docs link  │
│   alias              │  │   or warning         │
└──────────────────────┘  └──────────────────────┘
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
3. Install `cargo-nextest` for running tests
4. Run: `cargo nextest run --workspace`

This runs all Rust unit tests to ensure the code builds and tests pass before publishing.

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

#### Stage 6: Alias Versioned Docs

**Job**: `alias-versioned-docs` (depends on `stage`, `npm-publish`)

This stage creates a versioned subdomain alias for the documentation site, making docs for each release accessible at a version-specific URL (e.g., `v2-5-4.turborepo.dev`).

**Steps**:

1. Checkout the staging branch with full git history and tags
2. Read version from `version.txt` and transform to subdomain format:
   - `2.5.4` → `v2-5-4`
   - `2.7.5-canary.0` → `v2-7-5-canary-0`
3. Get the SHA for the version tag using `git rev-list`
4. Query Vercel API to find the deployment for that SHA
5. Use Vercel CLI to assign the subdomain alias

**Failure Handling**:

- If aliasing fails, a Slack notification is sent to `#team-turborepo`
- The release PR will include a prominent warning banner
- The release itself is **not blocked** - the PR will still be created

**Skipped during dry runs**: This stage only runs when `dry_run` is `false`.

**Required Secrets**:

| Secret              | Purpose                          |
| ------------------- | -------------------------------- |
| `TURBO_TOKEN`       | Vercel API authentication        |
| `VERCEL_ORG_ID`     | Vercel team ID                   |
| `VERCEL_PROJECT_ID` | Vercel project ID for turbo-site |

**Example URLs**:

| Version          | Subdomain URL                           |
| ---------------- | --------------------------------------- |
| `2.5.4`          | `https://v2-5-4.turborepo.dev`          |
| `2.7.5-canary.0` | `https://v2-7-5-canary-0.turborepo.dev` |
| `3.0.0`          | `https://v3-0-0.turborepo.dev`          |

#### Stage 7: Release PR

**For manual releases**: A PR is automatically created using the `thomaseizinger/create-pull-request` action. Merge it as soon as possible after publishing.

**For canary releases**: The canary workflow creates a PR with auto-merge enabled. The PR includes:

- A list of commits/PRs included since the last canary
- A link to versioned docs (if aliasing succeeded)

The PR body will include:

- **On success**: A link to the versioned docs (e.g., `https://v2-5-4.turborepo.dev`)
- **On failure**: A warning banner indicating the docs aliasing failed and needs manual intervention

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

| Variable                    | Purpose                                    | Example              |
| --------------------------- | ------------------------------------------ | -------------------- |
| `TURBO_VERSION`             | Version to release (read from version.txt) | `2.6.2`              |
| `TURBO_TAG`                 | npm dist-tag (read from version.txt)       | `latest` or `canary` |
| `CARGO_PROFILE_RELEASE_LTO` | Enable link-time optimization for Rust     | `true`               |
| `TURBO_BINARY_PATH`         | Override binary path (development only)    | `/path/to/turbo`     |

#### Rust Build Profile

The `release-turborepo` profile inherits from the release profile with debug symbol stripping enabled. Link-time optimization is enabled via the `CARGO_PROFILE_RELEASE_LTO=true` environment variable during the build.

See: `Cargo.toml` (release-turborepo profile)

#### Workflow Inputs Reference

| Input             | Type    | Required | Default      | Description                                                                                        |
| ----------------- | ------- | -------- | ------------ | -------------------------------------------------------------------------------------------------- |
| `increment`       | choice  | Yes      | `prerelease` | SemVer increment type: `prerelease`, `prepatch`, `preminor`, `premajor`, `patch`, `minor`, `major` |
| `dry_run`         | boolean | No       | `false`      | Skip npm publish and PR creation (test mode)                                                       |
| `tag-override`    | string  | No       | -            | Override npm dist-tag (e.g., for backports)                                                        |
| `ci-tag-override` | string  | No       | -            | Override npm tag for running tests (when recent release was faulty)                                |
| `sha`             | string  | No       | -            | Override SHA to release from (rarely used, mainly for debugging)                                   |

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

1. **Let canary releases happen automatically**: The hourly cron handles canary releases. No need to manually trigger `prerelease` for normal development.

2. **Use manual releases for stable versions**: When ready to promote to stable, manually trigger the release workflow with `patch`, `minor`, or `major`.

3. **Use dry run for testing**: When in doubt, use `dry_run: true` to test the entire pipeline without publishing.

4. **Monitor canary PRs**: Canary release PRs have auto-merge enabled, but check that they're merging successfully. If a canary PR fails to merge, investigate promptly.

5. **Check npm after publishing**: Verify that all packages were published correctly:

   ```bash
   npm view turbo@<version>
   npm view turbo-darwin-64@<version>
   npm view create-turbo@<version>
   # ... etc
   ```

6. **Handle failed releases carefully**: If a release fails mid-publish (some packages published, others not), document which packages were published and manually publish the rest if needed.

7. **Backporting**: Use `tag-override` when backporting fixes to older major versions. For example, releasing `2.5.3` when `main` is on `3.0.0`.

---

### Troubleshooting & Recovery

This section covers common failure scenarios and how to recover from them.

#### Canary Release Failed Mid-Publish

If a canary release fails after some packages were published but before others:

1. **Identify what was published**:

   ```bash
   VERSION="2.6.1-canary.5"  # The failed version
   for pkg in turbo turbo-darwin-64 turbo-darwin-arm64 turbo-linux-64 turbo-linux-arm64 turbo-windows-64 turbo-windows-arm64 create-turbo @turbo/codemod turbo-ignore @turbo/workspaces @turbo/gen eslint-plugin-turbo eslint-config-turbo @turbo/types; do
     npm view "$pkg@$VERSION" version 2>/dev/null && echo "✓ $pkg published" || echo "✗ $pkg NOT published"
   done
   ```

2. **Option A - Deprecate and re-release**: If few packages were published, deprecate them and trigger a new canary:

   ```bash
   # Deprecate the partial release
   npm deprecate turbo@2.6.1-canary.5 "Partial release, use 2.6.1-canary.6"
   npm deprecate turbo-darwin-64@2.6.1-canary.5 "Partial release, use 2.6.1-canary.6"
   # ... repeat for each published package

   # Merge any PR to main to trigger a new canary release
   ```

3. **Option B - Manual completion**: If most packages were published, manually publish the rest:

   ```bash
   cd cli
   # Ensure you're on the staging branch
   git checkout staging-2.6.1-canary.5
   # Publish missing packages manually
   npm publish ./path/to/package --tag canary
   ```

#### Canary PR Won't Auto-Merge

If a canary release PR is created but fails to auto-merge:

1. **Check branch protection**: Ensure required status checks are passing
2. **Check for conflicts**: The staging branch may have diverged from main
3. **Manual merge**: If checks pass, manually merge the PR via GitHub UI
4. **Cleanup if abandoned**: If you need to abandon the release:

   ```bash
   # Delete the staging branch
   git push origin --delete staging-2.6.1-canary.5
   # Close the PR via GitHub UI
   ```

#### Unexpected Repeated Releases

If canary releases keep firing when they shouldn't:

1. **Disable the workflow temporarily**:
   - Go to Actions → Release → "..." menu → Disable workflow

2. **Investigate the cause**:
   - Check if the skip detection is working: the `check-skip` job should skip when the latest commit is a release PR merge or when no relevant files changed since the last canary tag
   - Verify that release PR commit messages match the expected format: `release(turborepo): <version>`

3. **Fix and re-enable**:
   - Ensure the release PR title follows the expected format
   - Re-enable the workflow once the issue is resolved

#### Broken Package Released

If a canary release contains a critical bug:

1. **Deprecate immediately** (does NOT remove the package, just warns users):

   ```bash
   npm deprecate turbo@2.6.1-canary.5 "Critical bug in task scheduling, use 2.6.1-canary.6 or later"
   ```

2. **Cut a fix release**: Merge the fix to main; the next hourly canary run will pick it up automatically

3. **Unpublish (last resort, time-limited)**:
   - npm allows unpublish within 72 hours for packages with few downloads
   - Generally NOT recommended; deprecation is preferred

   ```bash
   # Only if absolutely necessary and within 72 hours
   npm unpublish turbo@2.6.1-canary.5
   ```

#### Staging Branch Cleanup

Staging branches (`staging-X.Y.Z`) are normally deleted when the PR merges. If orphaned branches accumulate:

```bash
# List orphaned staging branches
git fetch --prune
git branch -r | grep 'origin/staging-' | while read branch; do
  echo "Orphaned: $branch"
done

# Delete a specific orphaned branch
git push origin --delete staging-2.6.1-canary.5
```

#### Version Conflict

If two releases attempted to use the same version:

1. The second publish will fail with "cannot publish over existing version"
2. Check which release succeeded: `npm view turbo@X.Y.Z`
3. If needed, manually bump `version.txt` and re-trigger

#### Vercel Docs Aliasing Failed

If the versioned docs subdomain wasn't created:

1. Check the workflow logs for the specific error
2. Manually create the alias:

   ```bash
   # Find the deployment URL for the release commit
   vercel list turbo-site --scope=vercel -m githubCommitSha="<commit-sha>"

   # Create the alias
   vercel alias set <deployment-url> v2-6-1-canary-5.turborepo.dev --scope=vercel
   ```

3. A Slack notification is sent to `#team-turborepo` when this fails

---

### Security Considerations

The release pipeline handles sensitive operations (npm publishing, git tagging). Keep these security practices in mind:

1. **Commit messages are trusted input**: The skip detection reads the latest commit message via `git log`. This is safe because commits to `main` require PR approval, but never copy this pattern for workflows triggered by fork PRs.

2. **Version format is validated**: The pipeline validates that version strings match expected semver patterns before using them in shell commands.

3. **Secrets scope**: The canary workflow inherits secrets to the release workflow. Only maintainers with write access can trigger releases.

4. **OIDC publishing**: npm packages are published using GitHub's OIDC trusted publishing, which provides cryptographic provenance without storing long-lived tokens.

---

[1]: https://github.com/vercel/turborepo/actions/workflows/turborepo-release.yml
[2]: https://github.com/vercel/turborepo/blob/main/.github/workflows/turborepo-release.yml
[3]: https://github.com/apps/turbo-orchestrator
[4]: https://github.com/vercel/turborepo/blob/main/packages/turbo-repository/scripts/bump-version.sh
[5]: https://github.com/vercel/turborepo/actions/workflows/turborepo-library-release.yml
